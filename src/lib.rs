use futures::executor;
use once_cell::sync::Lazy;

static LOGGER_INIT: Lazy<()> = Lazy::new(|| wgpu_subscriber::initialize_default_subscriber(None));

pub fn init_adapter() -> wgpu::Adapter {
    Lazy::force(&LOGGER_INIT);

    let backend = match std::env::var("WGPU_BACKEND")
        .as_deref()
        .map(str::to_lowercase)
        .as_deref()
    {
        Ok("vk") => wgpu::BackendBit::VULKAN,
        Ok("dx12") => wgpu::BackendBit::DX12,
        Ok("dx11") => wgpu::BackendBit::DX11,
        Ok("metal") => wgpu::BackendBit::METAL,
        Ok("gl") => wgpu::BackendBit::GL,
        _ => wgpu::BackendBit::all(),
    };
    let instance = wgpu::Instance::new(backend);
    let adapter_future = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
    });
    executor::block_on(adapter_future).unwrap()
}

pub fn init_device() -> (wgpu::Device, wgpu::Queue) {
    let adapter = init_adapter();
    let device_future = adapter.request_device(&wgpu::DeviceDescriptor::default(), None);
    executor::block_on(device_future).unwrap()
}
