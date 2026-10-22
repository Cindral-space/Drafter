//! Display-free adapter-backed completed-upload variant of the drag benchmark.
use eframe::egui_wgpu::wgpu;
use std::{future::Future, sync::Arc, task::{Context, Poll, Wake, Waker}};
struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake { fn wake(self: Arc<Self>) { self.0.unpark(); } }
fn block_on<T>(future: impl Future<Output=T>) -> T {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop { match future.as_mut().poll(&mut context) { Poll::Ready(value) => return value, Poll::Pending => std::thread::park() } }
}
fn main() -> Result<(), String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter = block_on(instance.request_adapter(&Default::default())).map_err(|e|e.to_string())?;
    println!("Adapter: {:?}",adapter.get_info());
    let (device, queue) = block_on(adapter.request_device(&Default::default())).map_err(|e|e.to_string())?;
    let mut renderer = cindral_render::scene::SceneRenderer::new(&device,wgpu::TextureFormat::Rgba8Unorm,1);
    cindral_app::import_pushpull_benchmark::run(|update| {
        renderer.apply_update(&device,&queue,update);
        let submission = queue.submit([]);
        device.poll(wgpu::PollType::Wait { submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(5)) }).map_err(|e|e.to_string())?;
        Ok(())
    })
}
