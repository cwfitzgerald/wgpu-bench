@group(0) @binding(0)
var<storage, read> source: array<u32>;

@group(0) @binding(1)
var<storage, read_write> dest: array<u32>;

struct Pc {
    offset: u32,
}
var<push_constant> pc: Pc;

@compute @workgroup_size({{workgroup_size}})
fn main(@builtin(global_invocation_id) gid: vec3u) {
    dest[pc.offset + gid.x] = source[pc.offset + gid.x];
}