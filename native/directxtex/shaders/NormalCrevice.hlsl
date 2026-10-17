// Crevice term for normal-map AO: cav = -sum_k w_k * G_{s_k} * div(slope), the Frankot-Chellappa
// height minus its Gaussian blur, written as a sum of Gaussian blurs of the slope divergence.
// The host runs DivergenceCS once, then BlurXCS + BlurYAccumulateCS per scale; every pass wraps.
//
// Regenerate the vendored bytecode (vendored_shaders/NormalCrevice_*.inc) with:
//   fxc /nologo /T cs_5_0 /O3 /E <Entry> /Vn NormalCrevice_<Entry> /Fh vendored_shaders/NormalCrevice_<Entry>.inc shaders/NormalCrevice.hlsl
// for Entry in DivergenceCS, BlurXCS, BlurYAccumulateCS.

cbuffer Params : register(b0)
{
    uint g_width;
    uint g_height;
    uint g_radius;
    float g_sigma;
    float g_weight;
    float3 g_pad;
};

StructuredBuffer<uint> g_normals : register(t0); // RGBA8, R = x, G = y
StructuredBuffer<float> g_src : register(t1);
RWStructuredBuffer<float> g_dst : register(u0);

uint Wrap(int v, uint n)
{
    return (uint)((v % (int)n + (int)n) % (int)n);
}

// Tangent-space slope with green flipped: p = -nx/nz, q = -ny/nz.
float2 Slope(uint x, uint y)
{
    uint packed = g_normals[y * g_width + x];
    float nx = (float)(packed & 0xFF) / 255.0 * 2.0 - 1.0;
    float ny = (float)((packed >> 8) & 0xFF) / 255.0 * 2.0 - 1.0;
    float nz = sqrt(clamp(1.0 - nx * nx - ny * ny, 1e-4, 1.0));
    return float2(-nx / nz, -ny / nz);
}

[numthreads(8, 8, 1)]
void DivergenceCS(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= g_width || id.y >= g_height)
        return;
    int x = (int)id.x, y = (int)id.y;
    float dp = Slope(Wrap(x + 1, g_width), id.y).x - Slope(Wrap(x - 1, g_width), id.y).x;
    float dq = Slope(id.x, Wrap(y + 1, g_height)).y - Slope(id.x, Wrap(y - 1, g_height)).y;
    g_dst[id.y * g_width + id.x] = 0.5 * (dp + dq);
}

float Gauss(int i)
{
    return exp(-0.5 * (float)(i * i) / (g_sigma * g_sigma));
}

[numthreads(8, 8, 1)]
void BlurXCS(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= g_width || id.y >= g_height)
        return;
    float sum = 0.0, norm = 0.0;
    for (int i = -(int)g_radius; i <= (int)g_radius; ++i)
    {
        float w = Gauss(i);
        sum += w * g_src[id.y * g_width + Wrap((int)id.x + i, g_width)];
        norm += w;
    }
    g_dst[id.y * g_width + id.x] = sum / norm;
}

[numthreads(8, 8, 1)]
void BlurYAccumulateCS(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= g_width || id.y >= g_height)
        return;
    float sum = 0.0, norm = 0.0;
    for (int i = -(int)g_radius; i <= (int)g_radius; ++i)
    {
        float w = Gauss(i);
        sum += w * g_src[Wrap((int)id.y + i, g_height) * g_width + id.x];
        norm += w;
    }
    g_dst[id.y * g_width + id.x] += g_weight * sum / norm;
}
