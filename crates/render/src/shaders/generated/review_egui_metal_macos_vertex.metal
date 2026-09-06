#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct egui_params
{
    float2 screen_size;
    float2 egui_pad;
};

struct main0_out
{
    float2 v_uv [[user(locn0)]];
    float4 v_color [[user(locn1)]];
    float4 gl_Position [[position]];
};

struct main0_in
{
    float2 in_pos [[attribute(0)]];
    float2 in_uv [[attribute(1)]];
    float4 in_color [[attribute(2)]];
};

vertex main0_out main0(main0_in in [[stage_in]], constant egui_params& _27 [[buffer(0)]])
{
    main0_out out = {};
    out.gl_Position = float4(((2.0 * in.in_pos.x) / _27.screen_size.x) - 1.0, 1.0 - ((2.0 * in.in_pos.y) / _27.screen_size.y), 0.0, 1.0);
    out.v_uv = in.in_uv;
    out.v_color = in.in_color;
    return out;
}

