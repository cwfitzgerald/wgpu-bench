use std::time::{Duration, Instant};

use wgpu::{ImageCopyTexture, InstanceDescriptor};

fn main() {
    let instance = wgpu::Instance::new(InstanceDescriptor {
        backends: wgpu::util::backend_bits_from_env().unwrap_or(wgpu::Backends::all()),
        dx12_shader_compiler: wgpu::Dx12Compiler::Fxc,
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

    const QUERY_COUNT: u32 = 8;

    let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None,
        ty: wgpu::QueryType::Timestamp,
        count: QUERY_COUNT,
    });

    let size: usize = 1 << 28;
    let format = wgpu::TextureFormat::Rgba32Float;
    let format_size = format.block_size(None).unwrap() as usize;
    let texture_size = ((size / format_size) as f64).sqrt().round() as u32;

    println!(
        "data size {}",
        humansize::SizeFormatter::new(size, humansize::BINARY.decimal_places(2))
    );

    println!(
        "texture resolution {}, format size: {} B",
        humansize::SizeFormatter::new(texture_size, humansize::BINARY.decimal_places(2)),
        format_size,
    );

    let data_create_start = Instant::now();
    let data = vec![12u8; size];
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
        size: size as u64,
        usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let gpu_staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu staging buffer"),
        size: size as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let gpu_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu buffer"),
        size: size as u64,
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
        usage: wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
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

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.write_timestamp(&query_set, 0);
    encoder.copy_buffer_to_buffer(&staging_buffer, 0, &gpu_staging_buffer, 0, size as u64);
    encoder.write_timestamp(&query_set, 1);

    encoder.write_timestamp(&query_set, 2);
    encoder.copy_buffer_to_buffer(&gpu_staging_buffer, 0, &gpu_buffer, 0, size as u64);
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

    encoder.resolve_query_set(&query_set, 0..QUERY_COUNT, &query_resolve_buffer, 0);
    encoder.copy_buffer_to_buffer(
        &query_resolve_buffer,
        0,
        &query_copy_buffer,
        0,
        QUERY_COUNT as u64 * 8,
    );

    queue.submit(Some(encoder.finish()));
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

    let print_value = |name: &str, duration: Duration| {
        let speed = size as f64 / duration.as_secs_f64();
        println!(
            "{name}: {duration:>10.3?} {speed}/s",
            speed = humansize::ISizeFormatter::new(speed, humansize::BINARY.decimal_places(2))
        );
    };

    print_value("            vec![]", data_create_time);
    print_value("            memcpy", mapping_copy_time);
    print_value(
        " cpu -> gpu buffer",
        Duration::from_secs_f64(cpu_gpu_buffer_copy_time),
    );
    print_value(
        " gpu -> gpu buffer",
        Duration::from_secs_f64(gpu_gpu_buffer_copy_time),
    );
    print_value(
        "cpu -> gpu texture",
        Duration::from_secs_f64(cpu_gpu_texture_copy_time),
    );
    print_value(
        "gpu -> gpu texture",
        Duration::from_secs_f64(gpu_gpu_texture_copy_time),
    );
}
