// One instanced quad per cell background, decoration, cursor or glyph.
cbuffer Frame : register(b0) { float2 viewport; float2 pad; };

struct Inst {
    float2 pos : POS;     // top-left, pixels
    float2 size : SIZE;   // pixels
    float4 uv : UV;       // atlas u0 v0 u1 v1
    float4 color : COLOR; // straight RGBA
    uint kind : KIND;     // 0 solid, 1 mono glyph (tint by coverage), 2 color glyph (premultiplied), 3 image (straight)
};

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
    float4 color : COLOR;
    nointerpolation uint kind : KIND;
};

VsOut vs(Inst i, uint vid : SV_VertexID) {
    float2 c = float2(vid & 1, vid >> 1); // triangle strip corners
    float2 p = i.pos + c * i.size;
    VsOut o;
    o.pos = float4(p / viewport * float2(2, -2) + float2(-1, 1), 0, 1);
    o.uv = lerp(i.uv.xy, i.uv.zw, c);
    o.color = i.color;
    o.kind = i.kind;
    return o;
}

Texture2D atlas : register(t0);
Texture2D img : register(t1);
SamplerState samp : register(s0);

float4 ps(VsOut i) : SV_Target {
    if (i.kind == 0) return float4(i.color.rgb * i.color.a, i.color.a);
    if (i.kind == 3) {
        float4 t = img.Sample(samp, i.uv);
        float a = t.a * i.color.a;
        return float4(t.rgb * a, a);
    }
    float4 t = atlas.Sample(samp, i.uv);
    if (i.kind == 1) {
        float a = t.a * i.color.a;
        return float4(i.color.rgb * a, a);
    }
    return t * i.color.a;
}
