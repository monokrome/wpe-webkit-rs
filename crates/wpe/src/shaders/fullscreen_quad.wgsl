// Fullscreen quad shader for rendering WPE frame buffer

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Generate fullscreen quad from vertex index (2 triangles, 6 vertices)
    // Triangle 1: 0, 1, 2
    // Triangle 2: 2, 1, 3
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),  // bottom-left
        vec2<f32>(1.0, -1.0),   // bottom-right
        vec2<f32>(-1.0, 1.0),   // top-left
        vec2<f32>(-1.0, 1.0),   // top-left
        vec2<f32>(1.0, -1.0),   // bottom-right
        vec2<f32>(1.0, 1.0),    // top-right
    );

    var tex_coords = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),    // bottom-left
        vec2<f32>(1.0, 1.0),    // bottom-right
        vec2<f32>(0.0, 0.0),    // top-left
        vec2<f32>(0.0, 0.0),    // top-left
        vec2<f32>(1.0, 1.0),    // bottom-right
        vec2<f32>(1.0, 0.0),    // top-right
    );

    var output: VertexOutput;
    output.clip_position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.tex_coords = tex_coords[vertex_index];
    return output;
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_diffuse, s_diffuse, in.tex_coords);
}
