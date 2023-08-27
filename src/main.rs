use std::{
    num::NonZeroU64,
    time::{Duration, Instant},
};

use wgpu::{ImageCopyTexture, InstanceDescriptor};

const WORKGROUP_SIZE: u32 = 64;
const SIZE: usize = 1 << 28;

fn main() {
    let instance = wgpu::Instance::new(InstanceDescriptor {
        backends: wgpu::util::backend_bits_from_env().unwrap_or(wgpu::Backends::all()),
        dx12_shader_compiler: wgpu::Dx12Compiler::Fxc,
        ..InstanceDescriptor::default()
    });

    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .unwrap();

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            features: adapter.features(),
            limits: adapter.limits(),
        },
        None,
    ))
    .unwrap();

    assert!(
        device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
        "This example requires Features::TIMESTAMP_QUERY"
    );

    const QUERY_COUNT: u32 = 16;

    let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None,
        ty: wgpu::QueryType::Timestamp,
        count: QUERY_COUNT,
    });

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let format_size = format.block_size(None).unwrap() as usize;
    let texture_size = ((SIZE / format_size) as f64).sqrt().round() as u32;

    let info = adapter.get_info();
    println!("adapter: {}, {:?}", info.name, info.backend);

    println!(
        "data size {}",
        humansize::SizeFormatter::new(SIZE, humansize::BINARY.decimal_places(2))
    );

    println!(
        "texture resolution {}, format size: {} B",
        humansize::SizeFormatter::new(texture_size, humansize::BINARY.decimal_places(2)),
        format_size,
    );

    let compute_string =
        include_str!("comp.wgsl").replace("{{workgroup_size}}", &WORKGROUP_SIZE.to_string());

    let render_string =
        include_str!("blit.wgsl").replace("{{texture_size}}", &texture_size.to_string());

    let comp_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("comp.wgsl"),
        source: wgpu::ShaderSource::Wgsl(compute_string.into()),
    });

    let render_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit.wgsl"),
        source: wgpu::ShaderSource::Wgsl(render_string.into()),
    });

    let compute_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("buffer -> buffer bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
            ],
        });

    let render_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("buffer -> texture bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                },
                count: None,
            }],
        });

    let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("buffer -> buffer pll"),
        bind_group_layouts: &[&compute_bind_group_layout],
        push_constant_ranges: &[wgpu::PushConstantRange {
            stages: wgpu::ShaderStages::COMPUTE,
            range: 0..4,
        }],
    });

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("buffer -> texture pll"),
        bind_group_layouts: &[&render_bind_group_layout],
        push_constant_ranges: &[],
    });

    let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("buffer -> buffer compute"),
        layout: Some(&compute_pipeline_layout),
        module: &comp_sm,
        entry_point: "main",
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("buffer -> texture render"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &render_sm,
            entry_point: "vs_main",
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &render_sm,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
    });

    let data_create_start = Instant::now();
    let data = vec![12u8; SIZE];
    let data_create_time = data_create_start.elapsed();

    let query_resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("query resolve buffer"),
        size: QUERY_COUNT as u64 * 8,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let query_copy_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("query mapping buffer"),
        size: QUERY_COUNT as u64 * 8,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cpu staging buffer"),
        size: SIZE as u64,
        usage: wgpu::BufferUsages::MAP_WRITE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    let gpu_staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu staging buffer"),
        size: SIZE as u64,
        usage: wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    let gpu_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu buffer"),
        size: SIZE as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    let gpu_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("texture"),
        size: wgpu::Extent3d {
            width: texture_size,
            height: texture_size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    let gpu_texture_view = gpu_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let cpu_gpu_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cpu -> gpu bind group"),
        layout: &compute_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: staging_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: gpu_staging_buffer.as_entire_binding(),
            },
        ],
    });

    let gpu_gpu_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gpu -> gpu bind group"),
        layout: &compute_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: gpu_staging_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: gpu_buffer.as_entire_binding(),
            },
        ],
    });

    let cpu_gpu_render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cpu -> gpu render bind group"),
        layout: &render_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: staging_buffer.as_entire_binding(),
        }],
    });

    let gpu_gpu_render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gpu -> gpu render bind group"),
        layout: &render_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: gpu_staging_buffer.as_entire_binding(),
        }],
    });

    staging_buffer
        .slice(..)
        .map_async(wgpu::MapMode::Write, |_| ());
    device.poll(wgpu::MaintainBase::Wait);

    let mut mapping = staging_buffer.slice(..).get_mapped_range_mut();
    let mapping_copy_start = Instant::now();
    mapping.copy_from_slice(&data);
    let mapping_copy_time = mapping_copy_start.elapsed();

    drop(mapping);
    staging_buffer.unmap();

    for _ in 0..16 {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.write_timestamp(&query_set, 0);
        encoder.copy_buffer_to_buffer(&staging_buffer, 0, &gpu_staging_buffer, 0, SIZE as u64);
        encoder.write_timestamp(&query_set, 1);

        encoder.write_timestamp(&query_set, 2);
        encoder.copy_buffer_to_buffer(&gpu_staging_buffer, 0, &gpu_buffer, 0, SIZE as u64);
        encoder.write_timestamp(&query_set, 3);

        encoder.write_timestamp(&query_set, 4);
        encoder.copy_buffer_to_texture(
            wgpu::ImageCopyBuffer {
                buffer: &staging_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(texture_size * format_size as u32),
                    rows_per_image: None,
                },
            },
            ImageCopyTexture {
                texture: &gpu_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: texture_size,
                height: texture_size,
                depth_or_array_layers: 1,
            },
        );
        encoder.write_timestamp(&query_set, 5);

        encoder.write_timestamp(&query_set, 6);
        encoder.copy_buffer_to_texture(
            wgpu::ImageCopyBuffer {
                buffer: &gpu_staging_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(texture_size * format_size as u32),
                    rows_per_image: None,
                },
            },
            ImageCopyTexture {
                texture: &gpu_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: texture_size,
                height: texture_size,
                depth_or_array_layers: 1,
            },
        );
        encoder.write_timestamp(&query_set, 7);

        let workgroups = (SIZE / (WORKGROUP_SIZE as usize * 4)) as u32;

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cpu buffer -> buffer"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &query_set,
                beginning_of_pass_write_index: Some(8),
                end_of_pass_write_index: Some(9),
            }),
        });
        cpass.set_pipeline(&compute_pipeline);
        cpass.set_bind_group(0, &cpu_gpu_bind_group, &[]);
        for i in (0..workgroups).step_by(1 << 15) {
            let dispatches = (workgroups - i).min(1 << 15);
            cpass.set_push_constants(0, bytemuck::cast_slice(&[i]));
            cpass.dispatch_workgroups(dispatches, 1, 1);
        }
        drop(cpass);

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu buffer -> buffer"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &query_set,
                beginning_of_pass_write_index: Some(10),
                end_of_pass_write_index: Some(11),
            }),
        });
        cpass.set_pipeline(&compute_pipeline);
        cpass.set_bind_group(0, &gpu_gpu_bind_group, &[]);
        for i in (0..workgroups).step_by(1 << 15) {
            let dispatches = (workgroups - i).min(1 << 15);
            cpass.set_push_constants(0, bytemuck::cast_slice(&[i]));
            cpass.dispatch_workgroups(dispatches, 1, 1);
        }
        drop(cpass);

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cpu buffer -> texture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &gpu_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: true,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &query_set,
                beginning_of_pass_write_index: Some(12),
                end_of_pass_write_index: Some(13),
            }),
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&render_pipeline);
        rpass.set_bind_group(0, &cpu_gpu_render_bind_group, &[]);
        rpass.draw(0..3, 0..1);
        drop(rpass);

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gpu buffer -> texture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &gpu_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: true,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &query_set,
                beginning_of_pass_write_index: Some(14),
                end_of_pass_write_index: Some(15),
            }),
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&render_pipeline);
        rpass.set_bind_group(0, &gpu_gpu_render_bind_group, &[]);
        rpass.draw(0..3, 0..1);
        drop(rpass);

        encoder.resolve_query_set(&query_set, 0..QUERY_COUNT, &query_resolve_buffer, 0);
        encoder.copy_buffer_to_buffer(
            &query_resolve_buffer,
            0,
            &query_copy_buffer,
            0,
            QUERY_COUNT as u64 * 8,
        );

        queue.submit(Some(encoder.finish()));
    }
    query_copy_buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |_| ());
    device.poll(wgpu::MaintainBase::Wait);

    let query_copy_buffer_mapping = query_copy_buffer.slice(..).get_mapped_range();
    let values: &[u64] = bytemuck::cast_slice(&query_copy_buffer_mapping);

    let ticks_per_second = 1_000_000_000.0 / queue.get_timestamp_period() as f64;
    let cpu_gpu_buffer_copy_time = (values[1] - values[0]) as f64 / ticks_per_second;
    let gpu_gpu_buffer_copy_time = (values[3] - values[2]) as f64 / ticks_per_second;
    let cpu_gpu_texture_copy_time = (values[5] - values[4]) as f64 / ticks_per_second;
    let gpu_gpu_texture_copy_time = (values[7] - values[6]) as f64 / ticks_per_second;
    let cpu_gpu_buffer_shader_copy_time = (values[9] - values[8]) as f64 / ticks_per_second;
    let gpu_gpu_buffer_shader_copy_time = (values[11] - values[10]) as f64 / ticks_per_second;
    let cpu_gpu_texture_shader_copy_time = (values[13] - values[12]) as f64 / ticks_per_second;
    let gpu_gpu_texture_shader_copy_time = (values[15] - values[14]) as f64 / ticks_per_second;

    let print_value = |name: &str, duration: Duration| {
        let speed = SIZE as f64 / duration.as_secs_f64();
        println!(
            "{name}: {duration:>10.3?} {speed}/s",
            speed = humansize::ISizeFormatter::new(speed, humansize::BINARY.decimal_places(2))
        );
    };

    print_value("                   vec![]", data_create_time);
    print_value("                   memcpy", mapping_copy_time);
    print_value(
        "        cpu -> gpu buffer",
        Duration::from_secs_f64(cpu_gpu_buffer_copy_time),
    );
    print_value(
        "        gpu -> gpu buffer",
        Duration::from_secs_f64(gpu_gpu_buffer_copy_time),
    );
    print_value(
        "       cpu -> gpu texture",
        Duration::from_secs_f64(cpu_gpu_texture_copy_time),
    );
    print_value(
        "       gpu -> gpu texture",
        Duration::from_secs_f64(gpu_gpu_texture_copy_time),
    );
    print_value(
        " cpu -> gpu buffer shader",
        Duration::from_secs_f64(cpu_gpu_buffer_shader_copy_time),
    );
    print_value(
        " gpu -> gpu buffer shader",
        Duration::from_secs_f64(gpu_gpu_buffer_shader_copy_time),
    );
    print_value(
        "cpu -> gpu texture shader",
        Duration::from_secs_f64(cpu_gpu_texture_shader_copy_time),
    );
    print_value(
        "gpu -> gpu texture shader",
        Duration::from_secs_f64(gpu_gpu_texture_shader_copy_time),
    );
}
