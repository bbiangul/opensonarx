use image::{codecs::png::PngEncoder, ImageBuffer, ImageEncoder, Rgb};
use std::io::Cursor;

/// A template for generating a synthetic image.
pub struct ImageTemplate {
    pub label: String,
    pub description: String,
}

/// All available image templates.
pub fn all_templates() -> Vec<ImageTemplate> {
    vec![
        ImageTemplate {
            label: "red_square".into(),
            description: "A bright red square on black background".into(),
        },
        ImageTemplate {
            label: "blue_circle".into(),
            description: "A blue circle centered on black background".into(),
        },
        ImageTemplate {
            label: "green_triangle".into(),
            description: "A green triangle on black background".into(),
        },
        ImageTemplate {
            label: "sunset_gradient".into(),
            description: "A warm sunset gradient from orange to purple".into(),
        },
        ImageTemplate {
            label: "checkerboard".into(),
            description: "A black and white checkerboard pattern".into(),
        },
        ImageTemplate {
            label: "solid_black".into(),
            description: "A solid black image".into(),
        },
        ImageTemplate {
            label: "solid_white".into(),
            description: "A solid white image".into(),
        },
        ImageTemplate {
            label: "stripes".into(),
            description: "Horizontal alternating red and white stripes".into(),
        },
        ImageTemplate {
            label: "dots".into(),
            description: "White dots on a dark blue background".into(),
        },
        ImageTemplate {
            label: "diagonal_lines".into(),
            description: "Diagonal white lines on gray background".into(),
        },
    ]
}

const SIZE: u32 = 64;

/// Generate a PNG image for the given template label.
/// Returns (png_bytes, description).
pub fn generate_image(label: &str) -> (Vec<u8>, String) {
    let templates = all_templates();
    let tpl = templates
        .iter()
        .find(|t| t.label == label)
        .unwrap_or_else(|| panic!("Unknown image template: {label}"));

    let img = match label {
        "red_square" => draw_red_square(),
        "blue_circle" => draw_blue_circle(),
        "green_triangle" => draw_green_triangle(),
        "sunset_gradient" => draw_sunset_gradient(),
        "checkerboard" => draw_checkerboard(),
        "solid_black" => draw_solid(Rgb([0, 0, 0])),
        "solid_white" => draw_solid(Rgb([255, 255, 255])),
        "stripes" => draw_stripes(),
        "dots" => draw_dots(),
        "diagonal_lines" => draw_diagonal_lines(),
        _ => panic!("Unknown image template: {label}"),
    };

    let mut buf = Cursor::new(Vec::new());
    PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), SIZE, SIZE, image::ExtendedColorType::Rgb8)
        .expect("PNG encoding failed");

    (buf.into_inner(), tpl.description.clone())
}

fn draw_red_square() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        if (16..48).contains(&x) && (16..48).contains(&y) {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 0])
        }
    })
}

fn draw_blue_circle() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let cx = SIZE as f32 / 2.0;
    let cy = SIZE as f32 / 2.0;
    let r = 20.0f32;
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        if dx * dx + dy * dy <= r * r {
            Rgb([0, 80, 255])
        } else {
            Rgb([0, 0, 0])
        }
    })
}

fn draw_green_triangle() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        // Triangle: apex at (32, 8), base from (8, 56) to (56, 56)
        let fx = x as f32;
        let fy = y as f32;
        // Check if point is inside the triangle using barycentric test
        let (x1, y1) = (32.0f32, 8.0f32);
        let (x2, y2) = (8.0f32, 56.0f32);
        let (x3, y3) = (56.0f32, 56.0f32);
        let d = (y2 - y3) * (x1 - x3) + (x3 - x2) * (y1 - y3);
        let a = ((y2 - y3) * (fx - x3) + (x3 - x2) * (fy - y3)) / d;
        let b = ((y3 - y1) * (fx - x3) + (x1 - x3) * (fy - y3)) / d;
        let c = 1.0 - a - b;
        if a >= 0.0 && b >= 0.0 && c >= 0.0 {
            Rgb([0, 200, 0])
        } else {
            Rgb([0, 0, 0])
        }
    })
}

fn draw_sunset_gradient() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    ImageBuffer::from_fn(SIZE, SIZE, |_x, y| {
        let t = y as f32 / SIZE as f32;
        let r = (255.0 * (1.0 - t * 0.5)) as u8;
        let g = (140.0 * (1.0 - t)) as u8;
        let b = (60.0 + 120.0 * t) as u8;
        Rgb([r, g, b])
    })
}

fn draw_checkerboard() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let cell = 8u32;
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        if ((x / cell) + (y / cell)).is_multiple_of(2) {
            Rgb([255, 255, 255])
        } else {
            Rgb([0, 0, 0])
        }
    })
}

fn draw_solid(color: Rgb<u8>) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    ImageBuffer::from_pixel(SIZE, SIZE, color)
}

fn draw_stripes() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let stripe_h = 8u32;
    ImageBuffer::from_fn(SIZE, SIZE, |_x, y| {
        if (y / stripe_h).is_multiple_of(2) {
            Rgb([255, 0, 0])
        } else {
            Rgb([255, 255, 255])
        }
    })
}

fn draw_dots() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let spacing = 12u32;
    let radius = 3.0f32;
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        let cx = ((x + spacing / 2) / spacing) * spacing;
        let cy = ((y + spacing / 2) / spacing) * spacing;
        let dx = x as f32 - cx as f32;
        let dy = y as f32 - cy as f32;
        if dx * dx + dy * dy <= radius * radius {
            Rgb([255, 255, 255])
        } else {
            Rgb([0, 0, 40])
        }
    })
}

fn draw_diagonal_lines() -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    ImageBuffer::from_fn(SIZE, SIZE, |x, y| {
        if (x + y) % 8 < 2 {
            Rgb([255, 255, 255])
        } else {
            Rgb([128, 128, 128])
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_png() {
        for tpl in all_templates() {
            let (bytes, desc) = generate_image(&tpl.label);
            assert!(!bytes.is_empty(), "Empty PNG for {}", tpl.label);
            assert!(!desc.is_empty(), "Empty description for {}", tpl.label);
            // Verify it's a valid PNG by loading it
            let img = image::load_from_memory(&bytes)
                .unwrap_or_else(|e| panic!("Invalid PNG for {}: {e}", tpl.label));
            assert_eq!(img.width(), SIZE);
            assert_eq!(img.height(), SIZE);
        }
    }

    #[test]
    fn png_size_small() {
        for tpl in all_templates() {
            let (bytes, _) = generate_image(&tpl.label);
            assert!(
                bytes.len() < 8192,
                "{} PNG too large: {} bytes",
                tpl.label,
                bytes.len()
            );
        }
    }
}
