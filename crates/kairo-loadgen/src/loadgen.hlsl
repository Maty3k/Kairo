// SPDX-License-Identifier: GPL-3.0-or-later
// kairo-loadgen's shaders (docs/research/17-loadgen-bench/README.md sections 3.5 and 3.7).
// Written for Kairo; no third-party shader code. mix32 uses lowbias32's two multipliers
// (Chris Wellons' hash-prospector, public domain).
//
// Compiled at run time with D3DCompile from the inbox d3dcompiler_47.dll: vs_fullscreen as
// vs_5_0, ps_alu, ps_bw and ps_fill as ps_5_0, optimisation level 3, strict, warnings as errors.
// --detail off compiles with the macro KAIRO_DETAIL=0, which drops the grain.
//
// visual() is the picture, and src/visual.rs is its CPU reference, operation for operation:
// change one, change the other (a unit test holds their constants together, and Windows CI
// compares the GPU's read-back with the reference). Its limits are research 17 section 3.11's:
// sRGB codes 58 to 95, a colour cycle every 20 s, grey grain of +-6 codes scrolling at 120 px/s,
// nothing that steps in time.

#ifndef KAIRO_DETAIL
#define KAIRO_DETAIL 1
#endif

// The 32-byte constant buffer, rewritten every frame (src/workload.rs, FrameConstants).
cbuffer Frame : register(b0)
{
    float drift;        // frac(t / 20 s): the colour cycle, in cycles
    float scroll_px;    // (120 px/s * t) mod 8192: the grain's scroll
    uint count;         // --iters (alu) or --reads (bw)
    uint seed;          // the texture's seed (bw)
    float2 inv_size;    // 1 / the target's width and height
    uint2 tex_mask;     // the texture's width - 1 and height - 1 (bw)
};

// The bandwidth workload's texture: R32_UINT, filled once by ps_fill.
Texture2D<uint> big : register(t0);

// Two xorshift-multiply rounds: every input bit flips each output bit about half the time.
uint mix32(uint x)
{
    x ^= x >> 16; x *= 0x7feb352du; x ^= x >> 15; x *= 0x846ca68bu; x ^= x >> 16;
    return x;
}

// A lattice cell's value in [0, 1), from the top 24 bits of its hash.
float hash01(uint2 c)
{
    return float(mix32(c.x ^ mix32(c.y ^ 0x9e3779b9u)) >> 8) * (1.0 / 16777216.0);
}

// Smoothstep-interpolated value noise on 32 px cells; the lattice wraps every 256 cells
// (8192 px), where the scroll wraps too.
float value_noise(float2 p)
{
    float2 g = p / 32.0;
    float2 f = frac(g);
    float2 s = f * f * (3.0 - 2.0 * f);
    uint2 c = uint2(floor(g)) & 255u;
    float a = hash01(c);
    float b = hash01((c + uint2(1, 0)) & 255u);
    float d = hash01((c + uint2(0, 1)) & 255u);
    float e = hash01((c + 1u) & 255u);
    return lerp(lerp(a, b, s.x), lerp(d, e, s.x), s.y);
}

// The picture at SV_Position px: sRGB values for the UNORM target, codes 64 to 89 for the
// drifting colour field and +-6 codes of grey grain on top.
float3 visual(float2 px)
{
    float2 uv = px * inv_size;
    float a = 6.2831853 * (drift + 0.5 * uv.x + 0.25 * uv.y);
    float3 c = 0.30 + 0.05 * cos(a + float3(0.0, 2.0943951, 4.1887902));
    return c + KAIRO_DETAIL * (value_noise(float2(px.x + scroll_px, px.y)) - 0.5) * (12.0 / 255.0);
}

// One triangle over the whole target from SV_VertexID alone: (0,0) (2,0) (0,2) in UV, so
// there is no vertex buffer. Each band's draw is scissored to its rows.
float4 vs_fullscreen(uint id : SV_VertexID) : SV_Position
{
    float2 p = float2(float((id << 1) & 2u), float(id & 2u));
    return float4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}

// ALU: count steps per pixel, each a transcendental and a few ALU operations. The loop
// variable starts per pixel and the trip count is uniform, so nothing becomes scalar and
// nothing diverges. The work adds at most half a code, the same every frame.
float4 ps_alu(float4 pos : SV_Position) : SV_Target
{
    float x = frac(dot(pos.xy, float2(0.6180339, 0.4142135)));
    float acc = 0.0;
    [loop] for (uint k = 0u; k < count; ++k)
    {
        x = frac(x * 1.0001 + 0.5 * sin(x * 7.31 + float(k) * 0.01));
        acc += x;
    }
    return float4(visual(pos.xy) + (acc / float(max(count, 1u)) - 0.5) / 255.0, 1.0);
}

// Fills the texture once, in bands, from the seed: a render target before it is bound as a
// shader resource.
uint ps_fill(float4 pos : SV_Position) : SV_Target
{
    uint2 p = uint2(pos.xy);
    return mix32(p.x ^ mix32(p.y ^ seed));
}

// Bandwidth: count random loads per pixel, count a multiple of 4. Each address hashes the
// pixel, the step and the seed, never a loaded value, and each pass issues its four loads
// before it uses any, so four stay in flight. The work adds at most half a code.
float4 ps_bw(float4 pos : SV_Position) : SV_Target
{
    uint2 p = uint2(pos.xy);
    uint h = mix32(p.x ^ mix32(p.y ^ seed));
    uint acc = 0u;
    [loop] for (uint k = 0u; k < count; k += 4u)
    {
        uint a = mix32(h + k);
        uint b = mix32(h + k + 1u);
        uint c = mix32(h + k + 2u);
        uint d = mix32(h + k + 3u);
        uint va = big.Load(int3(int2(uint2(a, a >> 16) & tex_mask), 0));
        uint vb = big.Load(int3(int2(uint2(b, b >> 16) & tex_mask), 0));
        uint vc = big.Load(int3(int2(uint2(c, c >> 16) & tex_mask), 0));
        uint vd = big.Load(int3(int2(uint2(d, d >> 16) & tex_mask), 0));
        acc += (va ^ vb) + (vc ^ vd);
    }
    return float4(visual(pos.xy) + (float(acc & 255u) / 255.0 - 0.5) / 255.0, 1.0);
}
