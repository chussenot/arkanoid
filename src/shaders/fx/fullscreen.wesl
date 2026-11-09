// One oversized triangle covering the target: no vertex buffer.
struct FsOut {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

fn fullscreen(index: u32) -> FsOut {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var out: FsOut;
    out.position = vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2f(x, y);
    return out;
}
