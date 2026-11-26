// FLIP perceptual difference between two images, pure logic (no Bevy dependencies)
//
// This is LDR-FLIP from "FLIP: A Difference Evaluator for Alternating Images" (Andersson et al., 2020).
// Both images go through two pipelines:
// - color: contrast sensitivity filtering in an opponent space, then a distance in a perceptually uniform space
// - feature: edge and point detection on the luminance
// The feature difference then amplifies the color difference. The result is an error in [0, 1] per pixel.
//
// The contrast sensitivity and feature filters are (sums of) gaussians, so they are all applied as
// separable passes, and every per pixel step runs on bands of rows spread over the available cores.

use std::f32::consts::PI;
use std::f32::consts::SQRT_2;
use std::thread::available_parallelism;
use std::thread::scope;

use serde::{Deserialize, Serialize};

// Viewing condition of the reference implementation: a 0.7 m distance from a 0.7 m wide 4K monitor
pub const DEFAULT_PIXELS_PER_DEGREE: f32 = 67.;

// Pixels per degree of visual angle seen by someone at `distance_cm` from a screen of
// `diagonal_inches` showing `resolution` pixels (width, height)
pub fn pixels_per_degree(diagonal_inches: f32, distance_cm: f32, resolution: (f32, f32)) -> f32 {
    let diagonal_pixels = (resolution.0 * resolution.0 + resolution.1 * resolution.1).sqrt();
    let pixel_size_cm = diagonal_inches * 2.54 / diagonal_pixels;
    // Small angle: a degree covers distance * pi / 180 on the screen
    distance_cm / pixel_size_cm * PI / 180.
}

// Exponents and remapping points of the error redistribution
const QC: f32 = 0.7;
const QF: f32 = 0.5;
const PC: f32 = 0.4;
const PT: f32 = 0.95;
// Width of the feature detection filters, in degrees
const FEATURE_WIDTH: f32 = 0.082;

// Linear sRGB to CIE XYZ (D65)
const RGB_TO_XYZ: [[f32; 3]; 3] = [
    [10135552. / 24577794., 8788810. / 24577794., 4435075. / 24577794.],
    [2613072. / 12288897., 8788810. / 12288897., 887015. / 12288897.],
    [1425312. / 73733382., 8788810. / 73733382., 70074185. / 73733382.],
];

// Colors of the error map, from no error to the maximum one
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy, Default)]
pub enum ColorMap {
    // The one of the reference implementation: black, purple, orange, light yellow
    #[default]
    Magma,
    // Dark purple, blue, green, yellow
    Viridis,
}

impl ColorMap {
    pub const ALL: [ColorMap; 2] = [ColorMap::Magma, ColorMap::Viridis];

    pub fn label(&self) -> &'static str {
        match self {
            ColorMap::Magma => "Magma",
            ColorMap::Viridis => "Viridis",
        }
    }

    // Evenly spaced stops, colors in between are interpolated
    fn stops(&self) -> [[u8; 3]; 9] {
        match self {
            ColorMap::Magma => [
                [0x00, 0x00, 0x04],
                [0x1D, 0x11, 0x47],
                [0x51, 0x12, 0x7C],
                [0x82, 0x26, 0x81],
                [0xB6, 0x36, 0x79],
                [0xE6, 0x51, 0x64],
                [0xFB, 0x88, 0x61],
                [0xFE, 0xC2, 0x87],
                [0xFC, 0xFD, 0xBF],
            ],
            ColorMap::Viridis => [
                [0x44, 0x01, 0x54],
                [0x47, 0x2D, 0x7B],
                [0x3B, 0x52, 0x8B],
                [0x2C, 0x72, 0x8E],
                [0x21, 0x90, 0x8C],
                [0x27, 0xAD, 0x81],
                [0x5D, 0xC8, 0x63],
                [0xAA, 0xDC, 0x32],
                [0xFD, 0xE7, 0x25],
            ],
        }
    }

    fn color(&self, value: f32) -> [u8; 3] {
        let stops = self.stops();
        let position = value.clamp(0., 1.) * (stops.len() - 1) as f32;
        let index = (position as usize).min(stops.len() - 2);
        let fraction = position - index as f32;
        let mut color = [0u8; 3];
        for channel in 0..3 {
            let low = stops[index][channel] as f32;
            let high = stops[index + 1][channel] as f32;
            color[channel] = (low + (high - low) * fraction).round() as u8;
        }
        color
    }
}

// MARK: Parameters
// Everything that only depends on the viewing condition, shared by all the images of a comparison
pub struct FlipParams {
    // Linear RGB to XYZ divided by the white point (so white is 1, 1, 1), and back
    rgb_to_xyz: [[f32; 3]; 3],
    xyz_to_rgb: [[f32; 3]; 3],
    // Contrast sensitivity filters of the three opponent channels.
    // The blue-yellow one is a sum of two gaussians, mixed with these weights.
    achromatic: Vec<f32>,
    red_green: Vec<f32>,
    blue_yellow: [Vec<f32>; 2],
    blue_yellow_weights: [f32; 2],
    // Feature detection: a gaussian and its first (edge) and second (point) derivatives
    feature_smooth: Vec<f32>,
    edge: Vec<f32>,
    point: Vec<f32>,
    // Largest color distance considered, the one between pure green and pure blue
    color_max: f32,
}

impl FlipParams {
    pub fn new(pixels_per_degree: f32) -> Self {
        let pixels_per_degree = pixels_per_degree.max(1.);

        let mut rgb_to_xyz = RGB_TO_XYZ;
        for row in &mut rgb_to_xyz {
            let white = row[0] + row[1] + row[2];
            for value in row {
                *value /= white;
            }
        }
        let xyz_to_rgb = invert3(&rgb_to_xyz);

        // Contrast sensitivity functions are a1 * sqrt(pi / b1) * exp(-pi^2 * x^2 / b1) + the same with (a2, b2).
        // Only blue-yellow has a second term; its b1 is the widest gaussian of all and sets the radius.
        let (b_achromatic, b_red_green) = (0.0047, 0.0053);
        let (a1_blue_yellow, b1_blue_yellow, a2_blue_yellow, b2_blue_yellow) = (34.1f32, 0.04f32, 13.5f32, 0.025f32);
        let radius = (3. * (b1_blue_yellow / (2. * PI * PI)).sqrt() * pixels_per_degree).ceil() as usize;
        let (achromatic, _) = csf_kernel(radius, b_achromatic, pixels_per_degree);
        let (red_green, _) = csf_kernel(radius, b_red_green, pixels_per_degree);
        let (blue_yellow_1, sum_1) = csf_kernel(radius, b1_blue_yellow, pixels_per_degree);
        let (blue_yellow_2, sum_2) = csf_kernel(radius, b2_blue_yellow, pixels_per_degree);
        // Each 2D gaussian weighs its amplitude times its sum (the square of the 1D sum)
        let weight_1 = a1_blue_yellow * (PI / b1_blue_yellow).sqrt() * sum_1 * sum_1;
        let weight_2 = a2_blue_yellow * (PI / b2_blue_yellow).sqrt() * sum_2 * sum_2;
        let blue_yellow_weights = [weight_1 / (weight_1 + weight_2), weight_2 / (weight_1 + weight_2)];

        let (feature_smooth, edge, point) = feature_kernels(pixels_per_degree);

        let green = xyz_to_hunt_lab(mul3(&rgb_to_xyz, [0., 1., 0.]));
        let blue = xyz_to_hunt_lab(mul3(&rgb_to_xyz, [0., 0., 1.]));
        let color_max = hyab(green, blue).powf(QC);

        FlipParams {
            rgb_to_xyz,
            xyz_to_rgb,
            achromatic,
            red_green,
            blue_yellow: [blue_yellow_1, blue_yellow_2],
            blue_yellow_weights,
            feature_smooth,
            edge,
            point,
            color_max,
        }
    }
}

// 1D gaussian exp(-pi^2 * x^2 / b) sampled every pixel, normalized. Also returns its sum before normalization.
fn csf_kernel(radius: usize, b: f32, pixels_per_degree: f32) -> (Vec<f32>, f32) {
    let mut kernel = Vec::with_capacity(2 * radius + 1);
    let mut sum = 0.;
    for i in 0..2 * radius + 1 {
        let x = (i as f32 - radius as f32) / pixels_per_degree;
        let value = (-PI * PI * x * x / b).exp();
        kernel.push(value);
        sum += value;
    }
    for value in &mut kernel {
        *value /= sum;
    }
    (kernel, sum)
}

// Gaussian and its first and second derivatives. The derivatives are normalized so their positive
// weights sum to 1 and their negative ones to -1.
fn feature_kernels(pixels_per_degree: f32) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let sigma = 0.5 * FEATURE_WIDTH * pixels_per_degree;
    let radius = (3. * sigma).ceil() as usize;
    let mut smooth = Vec::with_capacity(2 * radius + 1);
    let mut edge = Vec::with_capacity(2 * radius + 1);
    let mut point = Vec::with_capacity(2 * radius + 1);
    for i in 0..2 * radius + 1 {
        let x = i as f32 - radius as f32;
        let gaussian = (-x * x / (2. * sigma * sigma)).exp();
        smooth.push(gaussian);
        edge.push(-x * gaussian);
        point.push((x * x / (sigma * sigma) - 1.) * gaussian);
    }

    let mut sum = 0.;
    for value in &smooth {
        sum += value;
    }
    for value in &mut smooth {
        *value /= sum;
    }
    for kernel in [&mut edge, &mut point] {
        let mut positive = 0.;
        let mut negative = 0.;
        for &value in kernel.iter() {
            if value > 0. {
                positive += value;
            } else {
                negative -= value;
            }
        }
        for value in kernel.iter_mut() {
            if *value > 0. {
                *value /= positive;
            } else if negative > 0. {
                *value /= negative;
            }
        }
    }
    (smooth, edge, point)
}

// MARK: Preprocessing
// What is kept of an image to compare it: the reference is preprocessed once for all the images compared to it
pub struct Preprocessed {
    pub width: usize,
    pub height: usize,
    // Filtered colors in L*a*b* with the Hunt adjustment, one plane per channel
    lab: [Vec<f32>; 3],
    // Strength of the edges and points
    edges: Vec<f32>,
    points: Vec<f32>,
}

// `rgb` holds the interleaved sRGB pixels in [0, 1], row by row
pub fn preprocess(width: usize, height: usize, mut rgb: Vec<f32>, params: &FlipParams) -> Preprocessed {
    let pixel_count = width * height;
    assert_eq!(rgb.len(), pixel_count * 3, "pixel buffer doesn't match the image size");

    // To the opponent space (luminance, red-green, blue-yellow), in place. This is YCxCz without its
    // offset and scales: the filters are normalized, so those would go through them unchanged.
    par_bands([&mut rgb[..]], width * 3, |_, [band]| {
        for pixel in band.as_chunks_mut::<3>().0 {
            let linear = [
                srgb_to_linear(pixel[0]),
                srgb_to_linear(pixel[1]),
                srgb_to_linear(pixel[2]),
            ];
            let xyz = mul3(&params.rgb_to_xyz, linear);
            pixel[0] = xyz[1];
            pixel[1] = xyz[0] - xyz[1];
            pixel[2] = xyz[1] - xyz[2];
        }
    });
    let mut luminance = vec![0f32; pixel_count];
    let mut red_green = vec![0f32; pixel_count];
    let mut blue_yellow = vec![0f32; pixel_count];
    for (i, pixel) in rgb.as_chunks::<3>().0.iter().enumerate() {
        luminance[i] = pixel[0];
        red_green[i] = pixel[1];
        blue_yellow[i] = pixel[2];
    }
    drop(rgb);

    // Feature pipeline, on the unfiltered luminance
    let smooth_rows = convolve_rows(&luminance, width, &params.feature_smooth);
    let edges = feature_strength(&luminance, &smooth_rows, width, &params.feature_smooth, &params.edge);
    let points = feature_strength(&luminance, &smooth_rows, width, &params.feature_smooth, &params.point);
    drop(smooth_rows);

    // Color pipeline: remove what the eye can't see at this distance...
    let mut l = convolve(&luminance, width, &params.achromatic);
    drop(luminance);
    let mut a = convolve(&red_green, width, &params.red_green);
    drop(red_green);
    let mut b = convolve(&blue_yellow, width, &params.blue_yellow[0]);
    let b_second = convolve(&blue_yellow, width, &params.blue_yellow[1]);
    drop(blue_yellow);

    // ...then go to L*a*b*, through RGB to clamp the colors filtering pushed out of gamut
    let weights = params.blue_yellow_weights;
    par_bands([&mut l[..], &mut a[..], &mut b[..]], width, |first_row, [l, a, b]| {
        let b_second = &b_second[first_row * width..];
        for i in 0..l.len() {
            let y = l[i];
            let blue_yellow = weights[0] * b[i] + weights[1] * b_second[i];
            let mut rgb = mul3(&params.xyz_to_rgb, [y + a[i], y, y - blue_yellow]);
            for value in &mut rgb {
                *value = value.clamp(0., 1.);
            }
            let lab = xyz_to_hunt_lab(mul3(&params.rgb_to_xyz, rgb));
            l[i] = lab[0];
            a[i] = lab[1];
            b[i] = lab[2];
        }
    });

    Preprocessed {
        width,
        height,
        lab: [l, a, b],
        edges,
        points,
    }
}

// Norm of the responses to a derivative filter along x and along y.
// `smooth_rows` is `src` already smoothed along x, shared by the edge and point detections.
fn feature_strength(src: &[f32], smooth_rows: &[f32], width: usize, smooth: &[f32], derivative: &[f32]) -> Vec<f32> {
    let mut along_x = convolve_columns(&convolve_rows(src, width, derivative), width, smooth);
    let along_y = convolve_columns(smooth_rows, width, derivative);
    par_bands([&mut along_x[..]], width, |first_row, [band]| {
        let along_y = &along_y[first_row * width..];
        for (value, y) in band.iter_mut().zip(along_y) {
            *value = (*value * *value + y * y).sqrt();
        }
    });
    along_x
}

// MARK: Error Map
// FLIP error of each pixel of `test`, in [0, 1]. Both images must have the same size.
pub fn flip_errors(reference: &Preprocessed, test: &Preprocessed, params: &FlipParams) -> Vec<f32> {
    assert_eq!(
        (reference.width, reference.height),
        (test.width, test.height),
        "FLIP needs images of the same size"
    );
    let width = reference.width;
    let color_max = params.color_max;
    let mut errors = vec![0f32; width * reference.height];
    par_bands([&mut errors[..]], width, |first_row, [band]| {
        let start = first_row * width;
        for (i, error) in band.iter_mut().enumerate() {
            let p = start + i;
            let reference_lab = [reference.lab[0][p], reference.lab[1][p], reference.lab[2][p]];
            let test_lab = [test.lab[0][p], test.lab[1][p], test.lab[2][p]];
            let distance = hyab(reference_lab, test_lab).powf(QC);
            // Small differences take most of the [0, 1] range, the large ones are squeezed at the top
            let color = if distance < PC * color_max {
                PT / (PC * color_max) * distance
            } else {
                PT + (distance - PC * color_max) / (color_max - PC * color_max) * (1. - PT)
            };

            let edge = (reference.edges[p] - test.edges[p]).abs();
            let point = (reference.points[p] - test.points[p]).abs();
            let feature = (edge.max(point) / SQRT_2).powf(QF).min(1.);

            *error = color.clamp(0., 1.).powf(1. - feature);
        }
    });
    errors
}

// Error map as RGBA8 pixels
pub fn colorize(errors: &[f32], color_map: ColorMap) -> Vec<u8> {
    let mut lut = [[0u8; 4]; 256];
    for (i, entry) in lut.iter_mut().enumerate() {
        let [r, g, b] = color_map.color(i as f32 / 255.);
        *entry = [r, g, b, 255];
    }
    let mut pixels = Vec::with_capacity(errors.len() * 4);
    for &error in errors {
        let index = (error.clamp(0., 1.) * 255.).round() as usize;
        pixels.extend_from_slice(&lut[index]);
    }
    pixels
}

// MARK: Color Math
fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn mul3(matrix: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * v[0] + matrix[0][1] * v[1] + matrix[0][2] * v[2],
        matrix[1][0] * v[0] + matrix[1][1] * v[1] + matrix[1][2] * v[2],
        matrix[2][0] * v[0] + matrix[2][1] * v[1] + matrix[2][2] * v[2],
    ]
}

// Inverse by cofactors, in f64 since the result is applied to every pixel
fn invert3(matrix: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0f64; 3]; 3];
    for row in 0..3 {
        for col in 0..3 {
            m[row][col] = matrix[row][col] as f64;
        }
    }
    let mut cofactors = [[0f64; 3]; 3];
    for (row, cofactors_row) in cofactors.iter_mut().enumerate() {
        for (col, cofactor) in cofactors_row.iter_mut().enumerate() {
            let (r1, r2) = ((row + 1) % 3, (row + 2) % 3);
            let (c1, c2) = ((col + 1) % 3, (col + 2) % 3);
            *cofactor = m[r1][c1] * m[r2][c2] - m[r1][c2] * m[r2][c1];
        }
    }
    let determinant = m[0][0] * cofactors[0][0] + m[0][1] * cofactors[0][1] + m[0][2] * cofactors[0][2];
    let mut inverse = [[0f32; 3]; 3];
    for row in 0..3 {
        for col in 0..3 {
            inverse[row][col] = (cofactors[col][row] / determinant) as f32;
        }
    }
    inverse
}

// XYZ relative to the white point to L*a*b*, with the Hunt adjustment:
// colors look less saturated when they are dark, so chroma is scaled by the lightness
fn xyz_to_hunt_lab(xyz: [f32; 3]) -> [f32; 3] {
    fn f(t: f32) -> f32 {
        const DELTA: f32 = 6. / 29.;
        if t > DELTA * DELTA * DELTA {
            t.cbrt()
        } else {
            t / (3. * DELTA * DELTA) + 4. / 29.
        }
    }
    let (fx, fy, fz) = (f(xyz[0]), f(xyz[1]), f(xyz[2]));
    let l = 116. * fy - 16.;
    [l, 500. * (fx - fy) * 0.01 * l, 200. * (fy - fz) * 0.01 * l]
}

// HyAB distance: city block on lightness, euclidean on chroma
fn hyab(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (da, db) = (a[1] - b[1], a[2] - b[2]);
    (a[0] - b[0]).abs() + (da * da + db * db).sqrt()
}

// MARK: Filtering
// Separable filter with the same kernel along x and y
fn convolve(src: &[f32], width: usize, kernel: &[f32]) -> Vec<f32> {
    convolve_columns(&convolve_rows(src, width, kernel), width, kernel)
}

// Filter along x. Pixels outside the image take the value of the closest edge pixel.
fn convolve_rows(src: &[f32], width: usize, kernel: &[f32]) -> Vec<f32> {
    let radius = kernel.len() / 2;
    let mut dst = vec![0f32; src.len()];
    par_bands([&mut dst[..]], width, |first_row, [band]| {
        // Columns whose whole window is inside the image, they don't need the clamping
        let inner_end = width.saturating_sub(radius);
        for (row_index, out) in band.chunks_exact_mut(width).enumerate() {
            let row = &src[(first_row + row_index) * width..][..width];
            for x in 0..width {
                let mut sum = 0.;
                if x >= radius && x < inner_end {
                    let window = &row[x - radius..x + radius + 1];
                    for i in 0..kernel.len() {
                        sum += kernel[i] * window[i];
                    }
                } else {
                    for (i, weight) in kernel.iter().enumerate() {
                        let source_x = (x + i).saturating_sub(radius).min(width - 1);
                        sum += weight * row[source_x];
                    }
                }
                out[x] = sum;
            }
        }
    });
    dst
}

// Filter along y, with the same edge handling. Accumulates whole rows at a time, which vectorizes well.
fn convolve_columns(src: &[f32], width: usize, kernel: &[f32]) -> Vec<f32> {
    let radius = kernel.len() / 2;
    let mut dst = vec![0f32; src.len()];
    if width == 0 {
        return dst;
    }
    let height = src.len() / width;
    par_bands([&mut dst[..]], width, |first_row, [band]| {
        for (row_index, out) in band.chunks_exact_mut(width).enumerate() {
            let y = first_row + row_index;
            for (i, &weight) in kernel.iter().enumerate() {
                let source_y = (y + i).saturating_sub(radius).min(height - 1);
                let row = &src[source_y * width..][..width];
                for x in 0..width {
                    out[x] += weight * row[x];
                }
            }
        }
    });
    dst
}

// Run `f(first_row, bands)` on horizontal bands of the planes, one band per core.
// All the planes have `row_len` values per row and the same number of rows.
fn par_bands<const N: usize>(planes: [&mut [f32]; N], row_len: usize, f: impl Fn(usize, [&mut [f32]; N]) + Sync) {
    if row_len == 0 || planes[0].is_empty() {
        return;
    }
    let rows = planes[0].len() / row_len;
    let threads = match available_parallelism() {
        Ok(count) => count.get(),
        Err(_) => 1,
    };
    let band_rows = rows.div_ceil(threads).max(1);
    let mut remaining = planes.map(|plane| plane.chunks_mut(band_rows * row_len));
    scope(|scope| {
        let mut first_row = 0;
        loop {
            let band = remaining.each_mut().map(|chunks| chunks.next());
            if band[0].is_none() {
                break;
            }
            let band = band.map(|plane| plane.unwrap());
            let f = &f;
            scope.spawn(move || f(first_row, band));
            first_row += band_rows;
        }
    });
}

// MARK: Tests
#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(width: usize, height: usize, color: [f32; 3]) -> Vec<f32> {
        let mut pixels = Vec::with_capacity(width * height * 3);
        for _ in 0..width * height {
            pixels.extend_from_slice(&color);
        }
        pixels
    }

    // Deterministic noise, so tests don't need a random generator
    fn noise(width: usize, height: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        let mut pixels = Vec::with_capacity(width * height * 3);
        for _ in 0..width * height * 3 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            pixels.push((state >> 8) as f32 / (1 << 24) as f32);
        }
        pixels
    }

    fn mean(values: &[f32]) -> f32 {
        let mut sum = 0.;
        for value in values {
            sum += value;
        }
        sum / values.len() as f32
    }

    #[test]
    fn pixels_per_degree_of_the_reference_setup() {
        // A 0.7 m wide 4K monitor has a 31.6 inches diagonal
        let value = pixels_per_degree(31.62, 70., (3840., 2160.));
        assert!((value - DEFAULT_PIXELS_PER_DEGREE).abs() < 0.1, "got {value}");
        // Twice as far, twice as many pixels in a degree
        let far = pixels_per_degree(31.62, 140., (3840., 2160.));
        assert!((far - 2. * value).abs() < 1e-3);
    }

    #[test]
    fn kernels_are_normalized() {
        let params = FlipParams::new(DEFAULT_PIXELS_PER_DEGREE);
        // Radii of the reference implementation at 67 pixels per degree
        assert_eq!(params.achromatic.len(), 21);
        assert_eq!(params.feature_smooth.len(), 19);
        for kernel in [
            &params.achromatic,
            &params.red_green,
            &params.blue_yellow[0],
            &params.blue_yellow[1],
            &params.feature_smooth,
        ] {
            assert!((kernel.iter().sum::<f32>() - 1.).abs() < 1e-5);
        }
        // Derivatives don't respond to flat areas
        assert!(params.edge.iter().sum::<f32>().abs() < 1e-5);
        assert!(params.point.iter().sum::<f32>().abs() < 1e-5);
        let weights = params.blue_yellow_weights;
        assert!((weights[0] + weights[1] - 1.).abs() < 1e-6);
    }

    #[test]
    fn matrix_inverse_round_trips() {
        let params = FlipParams::new(DEFAULT_PIXELS_PER_DEGREE);
        let color = [0.2, 0.5, 0.9];
        let back = mul3(&params.xyz_to_rgb, mul3(&params.rgb_to_xyz, color));
        for channel in 0..3 {
            assert!((back[channel] - color[channel]).abs() < 1e-5);
        }
        // White is (1, 1, 1) once divided by the white point, which is L = 100 without chroma
        let white = xyz_to_hunt_lab(mul3(&params.rgb_to_xyz, [1., 1., 1.]));
        assert!((white[0] - 100.).abs() < 1e-3);
        assert!(white[1].abs() < 1e-2 && white[2].abs() < 1e-2);
    }

    #[test]
    fn identical_images_have_no_error() {
        let params = FlipParams::new(DEFAULT_PIXELS_PER_DEGREE);
        let reference = preprocess(64, 48, noise(64, 48, 1), &params);
        let test = preprocess(64, 48, noise(64, 48, 1), &params);
        let errors = flip_errors(&reference, &test, &params);
        assert_eq!(errors.len(), 64 * 48);
        for error in errors {
            assert_eq!(error, 0.);
        }
    }

    #[test]
    fn black_against_white_is_close_to_the_maximum() {
        let params = FlipParams::new(DEFAULT_PIXELS_PER_DEGREE);
        let reference = preprocess(32, 32, uniform(32, 32, [0., 0., 0.]), &params);
        let test = preprocess(32, 32, uniform(32, 32, [1., 1., 1.]), &params);
        let errors = flip_errors(&reference, &test, &params);
        // Flat images have no feature, so this is the color error alone:
        // a distance of 100 on lightness, redistributed against the green to blue distance
        for &error in &errors {
            assert!(error > 0.95 && error < 1., "unexpected error {error}");
        }
        assert!((errors[0] - errors[errors.len() - 1]).abs() < 1e-5);
    }

    #[test]
    fn error_grows_with_the_difference() {
        let params = FlipParams::new(DEFAULT_PIXELS_PER_DEGREE);
        let reference = preprocess(32, 32, uniform(32, 32, [0.5, 0.5, 0.5]), &params);
        let near = preprocess(32, 32, uniform(32, 32, [0.52, 0.5, 0.5]), &params);
        let far = preprocess(32, 32, uniform(32, 32, [0.9, 0.5, 0.5]), &params);
        let near_error = mean(&flip_errors(&reference, &near, &params));
        let far_error = mean(&flip_errors(&reference, &far, &params));
        assert!(near_error > 0. && near_error < far_error && far_error <= 1.);
    }

    #[test]
    fn error_is_local() {
        let params = FlipParams::new(DEFAULT_PIXELS_PER_DEGREE);
        let (width, height) = (200, 100);
        let pixels = uniform(width, height, [0.5, 0.5, 0.5]);
        let mut changed = pixels.clone();
        // A bright square in the left half
        for y in 40..60 {
            for x in 20..40 {
                let p = (y * width + x) * 3;
                changed[p] = 1.;
                changed[p + 1] = 1.;
                changed[p + 2] = 1.;
            }
        }
        let reference = preprocess(width, height, pixels, &params);
        let test = preprocess(width, height, changed, &params);
        let errors = flip_errors(&reference, &test, &params);
        assert!(errors[50 * width + 30] > 0.3);
        assert_eq!(errors[50 * width + 150], 0.);
    }

    #[test]
    fn images_smaller_than_the_filters_are_handled() {
        let params = FlipParams::new(DEFAULT_PIXELS_PER_DEGREE);
        for (width, height) in [(1, 1), (3, 2), (5, 40), (40, 5)] {
            let reference = preprocess(width, height, noise(width, height, 1), &params);
            let test = preprocess(width, height, noise(width, height, 2), &params);
            let errors = flip_errors(&reference, &test, &params);
            assert_eq!(errors.len(), width * height);
            for error in errors {
                assert!((0. ..=1.).contains(&error));
            }
        }
    }

    #[test]
    fn color_maps_go_from_their_first_to_their_last_stop() {
        let viridis = colorize(&[0., 1.], ColorMap::Viridis);
        assert_eq!(viridis, [0x44, 0x01, 0x54, 255, 0xFD, 0xE7, 0x25, 255]);

        let pixels = colorize(&[0., 0.5, 1., 2.], ColorMap::Magma);
        assert_eq!(pixels.len(), 16);
        assert_eq!(pixels[0..4], [0x00, 0x00, 0x04, 255]);
        assert_eq!(pixels[8..12], [0xFC, 0xFD, 0xBF, 255]);
        // Out of range errors are clamped
        assert_eq!(pixels[12..16], pixels[8..12]);
    }
}
