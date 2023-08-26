@group(0) @binding(0)
var<storage, read> source: array<u32>;

@group(0) @binding(1)
var<storage, read_write> dest: array<u32>;

var<push_constant> offset: u32;

@compute @workgroup_size({{workgroup_size}})
fn main(@builtin(global_invocation_id) gid: vec3u) {
    dest[offset + gid.x] = source[offset + gid.x];
}