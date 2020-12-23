/*!
    Benchmark of CPU memory and descriptor allocators.
!*/

#[macro_use]
extern crate criterion;

use futures::executor;

fn init() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::BackendBit::all());
    let adapter_future = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
    });
    let adapter = executor::block_on(adapter_future).unwrap();
    let device_future = adapter.request_device(&wgpu::DeviceDescriptor::default(), None);
    executor::block_on(device_future).unwrap()
}

fn memory(c: &mut criterion::Criterion) {
    let (device, queue) = init();

    c.bench_function("Create and free a list of large GPU-local buffers", |b| {
        b.iter(|| {
            let mut buffers = Vec::new();
            for i in 0..7 {
                buffers.push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 1 << (16 + i),
                    usage: wgpu::BufferUsage::VERTEX,
                    mapped_at_creation: false,
                }));
            }
            buffers.clear();
            device.poll(wgpu::Maintain::Wait);
        })
    });

    c.bench_function("Run a number of write_buffer commands", |b| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1 << 25,
            usage: wgpu::BufferUsage::COPY_DST,
            mapped_at_creation: false,
        });
        let data = vec![0xFFu8; 1 << 25];
        b.iter(|| {
            for i in 0..10 {
                queue.write_buffer(&buffer, 0, &data[..1 << (16 + i)])
            }
            queue.submit(None);
        });
        device.poll(wgpu::Maintain::Wait);
    });
}

fn bind_group(c: &mut criterion::Criterion) {
    let (device, _) = init();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStage::all(),
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStage::all(),
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStage::all(),
                ty: wgpu::BindingType::Sampler {
                    filtering: true,
                    comparison: false,
                },
                count: None,
            },
        ],
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsage::UNIFORM,
        mapped_at_creation: false,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsage::SAMPLED,
    });
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());

    c.bench_function("Create and free a list of bind groups", |b| {
        b.iter(|| {
            let mut groups = Vec::new();
            for _ in 0..100 {
                groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                }));
            }
            groups.clear();
            device.poll(wgpu::Maintain::Wait);
        })
    });
}

criterion_group!(
    name = allocation;
    config = criterion::Criterion
        ::default()
        .warm_up_time(std::time::Duration::from_millis(200))
        .measurement_time(std::time::Duration::from_millis(1000))
        .sample_size(10);
    targets = memory, bind_group
);
criterion_main!(allocation);
