#import bevy_pbr::{
    forward_io::VertexOutput,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::PbrInput,
    pbr_fragment::pbr_input_from_vertex_output,
}
#import noisy_bevy::simplex_noise_2d
#import noisy_bevy::fbm_simplex_2d

// Hash for deterministic variation
fn hash22(p: vec2<f32>) -> vec2<f32> {
    let n = sin(dot(p, vec2<f32>(41.0, 289.0)));
    return fract(vec2<f32>(262144.0, 32768.0) * n) * 2.0 - 1.0;
}

// Brushed metal pattern - directional scratches
fn brushed_metal_pattern(p: vec2<f32>) -> f32 {
    // Primary scratch direction (along X axis for industrial look)
    let scratch_coord = p.x * 50.0 + p.y * 2.0;
    let fine_scratches = simplex_noise_2d(vec2<f32>(scratch_coord, p.y * 100.0)) * 0.5 + 0.5;

    // Larger surface imperfections
    let large_variation = fbm_simplex_2d(p * 0.05, 3, 2.0, 0.5) * 0.5 + 0.5;

    // Medium scale texture
    let medium_detail = simplex_noise_2d(p * 0.3) * 0.5 + 0.5;

    return fine_scratches * 0.4 + large_variation * 0.4 + medium_detail * 0.2;
}

// Surface wear pattern - affects roughness
fn wear_pattern(p: vec2<f32>) -> f32 {
    let warped_p = p + vec2<f32>(
        simplex_noise_2d(p * 0.02),
        simplex_noise_2d(p * 0.02 + vec2<f32>(100.0, 0.0))
    ) * 5.0;

    // Worn areas have different roughness
    let wear = fbm_simplex_2d(warped_p * 0.03, 4, 2.0, 0.5);
    return smoothstep(-0.2, 0.4, wear);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> @location(0) vec4<f32> {
    // World-space coordinates for infinite non-repeating pattern
    let world_pos = in.world_position.xz;

    // Cell-based variation
    let cell = floor(world_pos * 0.01);
    let hash_offset = hash22(cell) * 10.0;
    let varied_pos = world_pos + hash_offset;

    // Generate patterns
    let metal_pattern = brushed_metal_pattern(varied_pos);
    let wear = wear_pattern(varied_pos);

    // Base metal color - dark gunmetal
    let base_metal = vec3<f32>(0.04, 0.04, 0.045);  // Very dark metal
    let worn_metal = vec3<f32>(0.08, 0.08, 0.09);   // Slightly lighter where worn

    // Mix based on wear and pattern
    let color_variation = metal_pattern * 0.02;
    let base_color = mix(base_metal, worn_metal, wear * 0.5) + vec3<f32>(color_variation);

    // PBR material properties
    // Metallic: high (0.95) for metal ground
    let metallic = 0.95;

    // Roughness: varies with wear pattern (0.15 to 0.45)
    // Worn areas are rougher, polished areas are smoother
    let base_roughness = 0.15;
    let roughness = mix(base_roughness, 0.45, wear * 0.7 + metal_pattern * 0.3);

    // Reflectance: high for metal
    let reflectance = 0.9;

    // Initialize PBR input from vertex output
    var pbr_input: PbrInput = pbr_input_from_vertex_output(in, is_front, false);

    // Set material properties
    pbr_input.material.base_color = vec4<f32>(base_color, 1.0);
    pbr_input.material.metallic = metallic;
    pbr_input.material.perceptual_roughness = clamp(roughness, 0.089, 1.0);
    pbr_input.material.reflectance = reflectance;

    // Apply PBR lighting
    var color = apply_pbr_lighting(pbr_input);
    color = main_pass_post_lighting_processing(pbr_input, color);

    return color;
}
