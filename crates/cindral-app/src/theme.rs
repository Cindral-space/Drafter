//! UI-only preference; document colors and rendering never depend on it.
use eframe::egui;
/// Persisted application theme. Unknown/corrupt settings fall back to System.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Preference {
    /// Light application chrome.
    Light,
    /// Dark application chrome.
    Dark,
    /// Follow native system events, or Dark when unavailable.
    #[default]
    System,
}
impl Preference {
    pub(crate) fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
            Self::System => egui::ThemePreference::System,
        });
        ctx.request_repaint();
    }
}

pub(crate) fn error_background(v: &egui::Visuals) -> egui::Color32 {
    if v.dark_mode {
        egui::Color32::from_rgb(84, 34, 38)
    } else {
        egui::Color32::from_rgb(255, 220, 217)
    }
}

// winit 0.30 does not query the appearance portal on Wayland. Keep blocking
// D-Bus calls off the UI thread; use native winit events when no portal exists.
#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct SystemTheme {
    receiver: Option<std::sync::mpsc::Receiver<Option<egui::Theme>>>,
    stop: Option<std::sync::mpsc::Sender<()>>,
    current: Option<egui::Theme>,
}
#[cfg(target_os = "linux")]
impl SystemTheme {
    pub(crate) fn start(ctx: egui::Context) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (stop, shutdown) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok(connection) = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.method_timeout(std::time::Duration::from_secs(2)).build())
            else {
                return;
            };
            let Ok(proxy) = zbus::blocking::Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Settings",
            ) else {
                return;
            };
            let mut previous = None;
            loop {
                let current = proxy
                    .call::<_, _, zbus::zvariant::OwnedValue>(
                        "Read",
                        &("org.freedesktop.appearance", "color-scheme"),
                    )
                    .ok()
                    .and_then(|v| v.downcast_ref::<u32>().ok())
                    .and_then(|v| match v {
                        1 => Some(egui::Theme::Dark),
                        2 => Some(egui::Theme::Light),
                        _ => None,
                    });
                if previous != Some(current) {
                    if tx.send(current).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                    previous = Some(current);
                }
                if shutdown.recv_timeout(std::time::Duration::from_secs(2))
                    != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    return;
                }
            }
        });
        Self {
            receiver: Some(rx),
            stop: Some(stop),
            current: None,
        }
    }
    pub(crate) fn update(&mut self, input: &mut egui::RawInput) {
        if let Some(rx) = &self.receiver {
            for value in rx.try_iter() {
                self.current = value;
            }
        }
        if self.current.is_some() {
            input.system_theme = self.current;
        }
    }
}
#[cfg(target_os = "linux")]
impl Drop for SystemTheme {
    fn drop(&mut self) {
        if let Some(stop) = &self.stop {
            let _ = stop.send(());
        }
    }
}

fn luminance(color: egui::Color32) -> f32 {
    let c = color.to_array().map(|v| {
        let v = f32::from(v) / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}
fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
// Preserve semantic hue; alter only unreadable viewport ink, independently of UI theme.
pub(crate) fn overlay_color(background: [u8; 3], mut color: egui::Color32) -> egui::Color32 {
    let bg = egui::Color32::from_rgb(background[0], background[1], background[2]);
    for _ in 0..32 {
        if contrast(color, bg) >= 4.5 {
            break;
        }
        let [r, g, b, _] = color.to_array();
        let channel = |v: u8| {
            if luminance(bg) > 0.18 {
                (f32::from(v) * 0.8) as u8
            } else {
                v.saturating_add(((255 - u16::from(v)) / 5).max(1) as u8)
            }
        };
        color = egui::Color32::from_rgb(channel(r), channel(g), channel(b));
    }
    color
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        png_export::{self, Control, Options},
        workspace_ui, DrafterApp,
    };

    #[test]
    fn preferences_all_themes_roundtrip_and_corrupt_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        for theme in [Preference::Light, Preference::Dark, Preference::System] {
            let prefs = Options {
                theme,
                ..Default::default()
            };
            prefs.save(&path).unwrap();
            assert_eq!(Options::load(&path), prefs);
        }
        for bad in [
            "{interrupted",
            r#"{"theme":"unknown"}"#,
            r#"{"theme":4}"#,
            "null",
        ] {
            std::fs::write(&path, bad).unwrap();
            assert_eq!(Options::load(&path).theme, Preference::System);
        }
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(Options::load(&path).theme, Preference::System);
    }

    #[test]
    fn system_follows_input_and_missing_theme_is_dark() {
        let ctx = egui::Context::default();
        workspace_ui::configure_style(&ctx);
        Preference::System.apply(&ctx);
        for system in [Some(egui::Theme::Light), Some(egui::Theme::Dark), None] {
            let _ = ctx.run(
                egui::RawInput {
                    system_theme: system,
                    ..Default::default()
                },
                |_| {},
            );
            assert_eq!(ctx.theme(), system.unwrap_or(egui::Theme::Dark));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_updates_preserve_native_fallback_and_explicit_override() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut portal = SystemTheme {
            receiver: Some(rx),
            stop: None,
            current: None,
        };
        let ctx = egui::Context::default();
        crate::workspace_ui::configure_style(&ctx);
        for value in [Some(egui::Theme::Dark), Some(egui::Theme::Light), None] {
            tx.send(value).unwrap();
            let mut input = egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            };
            portal.update(&mut input);
            assert_eq!(input.system_theme, value.or(Some(egui::Theme::Light)));
            Preference::System.apply(&ctx);
            let _ = ctx.run(input.clone(), |_| {});
            assert_eq!(ctx.theme(), value.unwrap_or(egui::Theme::Light));
            Preference::Dark.apply(&ctx);
            let _ = ctx.run(input, |_| {});
            assert_eq!(ctx.theme(), egui::Theme::Dark);
        }
        let direct = zbus::zvariant::OwnedValue::from(1_u32);
        let nested = zbus::zvariant::Value::new(zbus::zvariant::Value::U32(2))
            .try_to_owned()
            .unwrap();
        assert_eq!(direct.downcast_ref::<u32>().unwrap(), 1);
        assert_eq!(nested.downcast_ref::<u32>().unwrap(), 2);
    }

    #[test]
    fn palette_contrast_including_disabled_errors_and_overlays() {
        let ctx = egui::Context::default();
        workspace_ui::configure_style(&ctx);
        for theme in [Preference::Light, Preference::Dark] {
            theme.apply(&ctx);
            let v = ctx.style().visuals.clone();
            let backgrounds = [
                v.panel_fill,
                v.window_fill,
                v.extreme_bg_color,
                v.selection.bg_fill,
            ];
            for bg in backgrounds {
                for fg in [
                    v.text_color(),
                    v.weak_text_color(),
                    v.selection.stroke.color,
                    v.error_fg_color,
                    v.warn_fg_color,
                ] {
                    assert!(contrast(fg, bg) >= 4.5, "{theme:?}: {fg:?} / {bg:?}");
                }
                let disabled = v.text_color().to_array();
                let back = bg.to_array();
                let blend = |i: usize| {
                    (f32::from(disabled[i]) * v.disabled_alpha
                        + f32::from(back[i]) * (1. - v.disabled_alpha)) as u8
                };
                assert!(contrast(egui::Color32::from_rgb(blend(0), blend(1), blend(2)), bg) >= 4.5);
            }
            assert!(contrast(v.error_fg_color, error_background(&v)) >= 4.5);
            assert!(contrast(v.text_color(), error_background(&v)) >= 4.5);
            assert!(contrast(v.weak_text_color(), error_background(&v)) >= 4.5);
            for widget in [
                &v.widgets.inactive,
                &v.widgets.hovered,
                &v.widgets.active,
                &v.widgets.open,
            ] {
                assert!(contrast(v.text_color(), widget.bg_fill) >= 4.5);
                assert!(contrast(widget.bg_stroke.color, widget.bg_fill) >= 3.);
            }
            for bg in [
                [237, 240, 245],
                [255; 3],
                [25, 30, 40],
                [0; 3],
                [90, 130, 185],
                [80, 90, 65],
            ] {
                for color in [
                    [255, 184, 68],
                    [250, 202, 114],
                    [100, 164, 190],
                    [85, 185, 210],
                    [113, 226, 203],
                    [128; 3],
                    [255, 175, 55],
                ] {
                    let c =
                        overlay_color(bg, egui::Color32::from_rgb(color[0], color[1], color[2]));
                    assert!(contrast(c, egui::Color32::from_rgb(bg[0], bg[1], bg[2])) >= 4.5);
                }
            }
        }
    }

    #[test]
    fn switch_changes_palette_preserves_document_and_export_bytes() {
        let ctx = egui::Context::default();
        workspace_ui::configure_style(&ctx);
        let mut app = DrafterApp::default();
        app.execute(cindral_core::Command::AddPolyline {
            points: vec![
                cindral_core::DVec3::ZERO,
                cindral_core::DVec3::X,
                cindral_core::DVec3::Y,
            ],
            closed: true,
        })
        .unwrap();
        app.set_view_style(cindral_core::ViewStyle {
            background: [205, 215, 225],
            sky: Some([90, 130, 185]),
            ground: Some([80, 90, 65]),
            ..Default::default()
        });
        let original = serde_json::to_vec(&app.file_values().document).unwrap();
        let presentation = app.presentation.clone();
        let dir = tempfile::tempdir().unwrap();
        let mut previous_palette = None;
        let mut previous_bytes = None;
        for theme in [Preference::Light, Preference::Dark, Preference::System] {
            theme.apply(&ctx);
            let palette = ctx.style().visuals.panel_fill;
            if let Some(previous) = previous_palette {
                assert_ne!(palette, previous);
            }
            previous_palette = if theme == Preference::Light {
                Some(palette)
            } else {
                None
            };
            let bundle = app.file_values();
            assert_eq!(serde_json::to_vec(&bundle.document).unwrap(), original);
            assert_eq!(bundle.presentation, presentation);
            let gltf = dir.path().join("model.gltf");
            let dae = dir.path().join("model.dae");
            cindral_io::export_gltf(&gltf, &bundle).unwrap();
            cindral_io::export_dae(
                &dae,
                &bundle,
                &cindral_io::DaeOptions {
                    force: true,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let options = Options {
                width: 2,
                height: 2,
                aa: png_export::Aa::Off,
                theme,
                ..Default::default()
            };
            let raw = vec![0; 2 * 2 * 4];
            let pixels = png_export::resolve(
                &raw,
                &options,
                &app.camera,
                &bundle.presentation.style,
                false,
                &Control::default(),
            )
            .unwrap();
            let png = dir.path().join("model.png");
            png_export::write_png(
                &png,
                &options,
                &pixels,
                "model.cdraft",
                true,
                &Control::default(),
            )
            .unwrap();
            let bytes = [
                std::fs::read(gltf).unwrap(),
                std::fs::read(dae).unwrap(),
                std::fs::read(png).unwrap(),
            ];
            if let Some(previous) = &previous_bytes {
                assert_eq!(&bytes, previous);
            }
            previous_bytes = Some(bytes);
        }
    }
}
