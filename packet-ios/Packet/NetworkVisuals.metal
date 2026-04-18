#include <metal_stdlib>
using namespace metal;

// A dynamic, cyber-fluid background that reacts to the tunnel state
[[ stitchable ]] half4 cyberBackground(float2 position, half4 currentColor, float time, float2 size, float isActive) {
    // Normalize coordinates
    float2 uv = position / size;
    
    // Create moving, intersecting network waves
    float wave1 = sin(uv.x * 6.0 + time * 1.5) * 0.5 + 0.5;
    float wave2 = cos(uv.y * 5.0 - time * 1.2) * 0.5 + 0.5;
    float wave3 = sin((uv.x + uv.y) * 4.0 + time) * 0.5 + 0.5;
    
    // Calculate state intensity (1.0 = Active, 0.2 = Idle/Error)
    float intensity = mix(0.15, 1.0, isActive);
    
    // Base colors (Dark premium aesthetic)
    // Active: Glowing cyan/blue. Idle: Subtle dark greys
    half r = half(0.02 * intensity);
    half g = half((0.1 + wave1 * 0.15) * intensity);
    half b = half((0.2 + wave2 * 0.25 + wave3 * 0.1) * intensity);
    
    return half4(r, g, b, 1.0);
}