//! 缩略图工具栏的按钮图标：在代码里按系统小图标尺寸现画，不带位图资源。
//! 任意 DPI 下都是整像素对齐的清晰图形，换主题色也只是换一个颜色重画

/// 每个像素按 4×4 子采样估算覆盖率，足够把斜边抗锯齿
const SUBSAMPLES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
}

type Point = (f32, f32);

/// 图形都定义在单位方块里（左上为原点），四周留出约 20% 空白，与系统自带按钮的视觉大小相当
fn shapes(glyph: Glyph) -> Vec<Vec<Point>> {
    let next = vec![
        vec![(0.22, 0.22), (0.62, 0.5), (0.22, 0.78)],
        vec![(0.64, 0.22), (0.76, 0.22), (0.76, 0.78), (0.64, 0.78)],
    ];
    match glyph {
        Glyph::Play => vec![vec![(0.3, 0.2), (0.8, 0.5), (0.3, 0.8)]],
        Glyph::Pause => vec![
            vec![(0.26, 0.2), (0.42, 0.2), (0.42, 0.8), (0.26, 0.8)],
            vec![(0.58, 0.2), (0.74, 0.2), (0.74, 0.8), (0.58, 0.8)],
        ],
        Glyph::Next => next,
        Glyph::Previous => next
            .into_iter()
            .map(|shape| shape.into_iter().map(|(x, y)| (1.0 - x, y)).collect())
            .collect(),
    }
}

/// 点是否落在凸多边形内（含边上）；不依赖顶点绕向，镜像出来的图形也能直接用
fn inside_convex(point: Point, polygon: &[Point]) -> bool {
    let mut sign = 0.0_f32;
    for (index, &(ax, ay)) in polygon.iter().enumerate() {
        let (bx, by) = polygon[(index + 1) % polygon.len()];
        let cross = (bx - ax) * (point.1 - ay) - (by - ay) * (point.0 - ax);
        if cross.abs() <= f32::EPSILON {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// 画成 `size × size` 的 BGRA 像素（自上而下逐行，非预乘 alpha，图标位图的标准格式）
pub fn render(glyph: Glyph, size: usize, rgb: [u8; 3]) -> Vec<u8> {
    let shapes = shapes(glyph);
    let mut pixels = vec![0_u8; size * size * 4];
    if size == 0 {
        return pixels;
    }
    let step = 1.0 / (size * SUBSAMPLES) as f32;
    for row in 0..size {
        for column in 0..size {
            let mut covered = 0;
            for sy in 0..SUBSAMPLES {
                for sx in 0..SUBSAMPLES {
                    let point = (
                        ((column * SUBSAMPLES + sx) as f32 + 0.5) * step,
                        ((row * SUBSAMPLES + sy) as f32 + 0.5) * step,
                    );
                    if shapes.iter().any(|shape| inside_convex(point, shape)) {
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let offset = (row * size + column) * 4;
            let alpha = covered * 255 / (SUBSAMPLES * SUBSAMPLES);
            pixels[offset..offset + 4].copy_from_slice(&[rgb[2], rgb[1], rgb[0], alpha as u8]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: [u8; 3] = [10, 20, 30];

    fn alpha(pixels: &[u8], size: usize, x: usize, y: usize) -> u8 {
        pixels[(y * size + x) * 4 + 3]
    }

    fn coverage(pixels: &[u8]) -> usize {
        pixels.chunks(4).map(|pixel| usize::from(pixel[3])).sum()
    }

    #[test]
    fn renders_bgra_rows_of_the_requested_size() {
        for size in [16, 20, 24, 32] {
            assert_eq!(render(Glyph::Play, size, INK).len(), size * size * 4);
        }
        assert!(render(Glyph::Play, 0, INK).is_empty());
    }

    #[test]
    fn stores_colour_as_bgr_with_straight_alpha() {
        let pixels = render(Glyph::Pause, 16, INK);
        let solid = pixels
            .chunks(4)
            .find(|pixel| pixel[3] == 255)
            .expect("pause bars have solid pixels");
        assert_eq!(&solid[..3], &[30, 20, 10]);
    }

    #[test]
    fn keeps_a_transparent_margin_around_every_glyph() {
        for glyph in [Glyph::Previous, Glyph::Play, Glyph::Pause, Glyph::Next] {
            let size = 20;
            let pixels = render(glyph, size, INK);
            for edge in 0..size {
                for (x, y) in [(edge, 0), (edge, size - 1), (0, edge), (size - 1, edge)] {
                    assert_eq!(
                        alpha(&pixels, size, x, y),
                        0,
                        "{glyph:?} touches the border at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn anti_aliases_the_slanted_edges_of_the_play_triangle() {
        let pixels = render(Glyph::Play, 24, INK);
        assert!(pixels.chunks(4).any(|pixel| pixel[3] > 0 && pixel[3] < 255));
    }

    #[test]
    fn mirrors_next_into_previous() {
        let size = 24;
        let next = render(Glyph::Next, size, INK);
        let previous = render(Glyph::Previous, size, INK);
        for y in 0..size {
            for x in 0..size {
                assert_eq!(
                    alpha(&next, size, x, y),
                    alpha(&previous, size, size - 1 - x, y)
                );
            }
        }
    }

    #[test]
    fn draws_distinct_glyphs_for_play_and_pause() {
        let play = render(Glyph::Play, 24, INK);
        let pause = render(Glyph::Pause, 24, INK);
        assert_ne!(play, pause);
        // 暂停键中间是空隙，播放三角在中间是实心的
        assert_eq!(alpha(&pause, 24, 12, 12), 0);
        assert_eq!(alpha(&play, 24, 12, 12), 255);
        assert!(coverage(&play) > 0 && coverage(&pause) > 0);
    }
}
