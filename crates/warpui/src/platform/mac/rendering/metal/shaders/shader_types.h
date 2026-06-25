#ifndef shader_types_h
#define shader_types_h

// Forward-declare the SIMD vector types we need instead of including
// <simd/simd.h>.  The system header transitively pulls in clang intrinsic
// headers (avx512fp16intrin.h, amxavx512intrin.h, avx10_2*intrin.h, …)
// that define _Float16 / __bf16 types which bindgen cannot process.  Each
// Xcode release adds more of these headers, so suppressing them one by one
// with -D flags in build.rs is a maintenance burden.
//
// Metal Shading Language compilers treat vector_float{2,4} as built-in
// types — the forward declarations are only consumed by bindgen (C/C++
// parser), so Metal compilation is unaffected.
#ifndef __METAL_VERSION__
typedef float vector_float2 __attribute__((__ext_vector_type__(2)));
typedef float vector_float4 __attribute__((__ext_vector_type__(4)));
#endif

typedef struct {
  vector_float2 viewport_size;
} Uniforms;

typedef struct {
  vector_float2 origin;
  vector_float2 size;
  float corner_radius_top_left;
  float corner_radius_top_right;
  float corner_radius_bottom_left;
  float corner_radius_bottom_right;
  float border_top;
  float border_right;
  float border_bottom;
  float border_left;
  vector_float2 background_start;
  vector_float2 background_end;
  vector_float4 background_start_color;
  vector_float4 background_end_color;
  vector_float2 border_start;
  vector_float2 border_end;
  vector_float4 border_start_color;
  vector_float4 border_end_color;
  vector_float4 icon_color;
  int is_icon;
  vector_float2 drop_shadow_offsets;
  vector_float4 drop_shadow_color;
  float drop_shadow_sigma;
  float drop_shadow_padding_factor;
  float dash_length;
  vector_float2 gap_lengths;
} PerRectUniforms;

typedef struct {
  vector_float2 origin;
  vector_float2 size;
  float uv_left;
  float uv_top;
  float uv_width;
  float uv_height;
  float fade_start;
  float fade_end;
  vector_float4 color;
  int is_emoji;
} PerGlyphUniforms;

#endif // shader_types_h
