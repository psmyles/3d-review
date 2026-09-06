#pragma clang diagnostic ignored "-Wmissing-prototypes"
#pragma clang diagnostic ignored "-Wmissing-braces"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

template<typename T, size_t Num>
struct spvUnsafeArray
{
    T elements[Num ? Num : 1];
    
    thread T& operator [] (size_t pos) thread
    {
        return elements[pos];
    }
    constexpr const thread T& operator [] (size_t pos) const thread
    {
        return elements[pos];
    }
    
    device T& operator [] (size_t pos) device
    {
        return elements[pos];
    }
    constexpr const device T& operator [] (size_t pos) const device
    {
        return elements[pos];
    }
    
    constexpr const constant T& operator [] (size_t pos) const constant
    {
        return elements[pos];
    }
    
    threadgroup T& operator [] (size_t pos) threadgroup
    {
        return elements[pos];
    }
    constexpr const threadgroup T& operator [] (size_t pos) const threadgroup
    {
        return elements[pos];
    }
};

struct face_params
{
    float4 forward;
    float4 right;
    float4 up;
    float4 params;
};

constant spvUnsafeArray<float2, 3> _24 = spvUnsafeArray<float2, 3>({ float2(-1.0), float2(3.0, -1.0), float2(-1.0, 3.0) });

struct main0_out
{
    float3 v_local_dir [[user(locn0)]];
    float2 v_uv [[user(locn1)]];
    float4 gl_Position [[position]];
};

static inline __attribute__((always_inline))
float2 fullscreen_corner(thread const int& index)
{
    return _24[index];
}

vertex main0_out main0(constant face_params& _57 [[buffer(0)]], uint gl_VertexIndex [[vertex_id]])
{
    main0_out out = {};
    int param = int(gl_VertexIndex);
    float2 _36 = fullscreen_corner(param);
    float _47 = _36.x;
    float _48 = _36.y;
    out.gl_Position = float4(_47, _48, 0.0, 1.0);
    out.v_local_dir = (_57.forward.xyz + (_57.right.xyz * _47)) + (_57.up.xyz * _48);
    out.v_uv = float2((_47 * 0.5) + 0.5, 0.5 - (_48 * 0.5));
    return out;
}

