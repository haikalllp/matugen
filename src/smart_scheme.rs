use std::{
    fs::{create_dir_all, read_to_string, File},
    io::{BufWriter, Write},
    path::Path,
};

use color_eyre::Report;
use image::{imageops::FilterType, DynamicImage, GenericImageView, ImageReader};
use material_colors::hct::Hct;
use serde::{Deserialize, Serialize};

use crate::color::format::argb_from_rgb;
use crate::{
    scheme::{SchemeTypes, SchemesEnum},
    util::config::{get_proj_path, ProjectDirsTypes},
};
use colorsys::Rgb;

pub struct SmartOpts {
    pub mode: SchemesEnum,
    pub variant: SchemeTypes,
}

#[derive(Serialize, Deserialize)]
struct SmartCache {
    mode: String,
    variant: String,
}

fn calc_colourfulness(image: &DynamicImage) -> f64 {
    let rgb_image = image.to_rgb8();
    let pixels = rgb_image.pixels();

    let mut rg_sum = 0.0;
    let mut yb_sum = 0.0;
    let mut rg_sq_sum = 0.0;
    let mut yb_sq_sum = 0.0;
    let mut count = 0u64;

    for pixel in pixels {
        let r = pixel[0] as f64;
        let g = pixel[1] as f64;
        let b = pixel[2] as f64;

        let rg = (r - g).abs();
        let yb = (0.5 * (r + g) - b).abs();

        rg_sum += rg;
        yb_sum += yb;
        rg_sq_sum += rg * rg;
        yb_sq_sum += yb * yb;
        count += 1;
    }

    if count == 0 {
        return 0.0;
    }

    let mean_rg = rg_sum / count as f64;
    let mean_yb = yb_sum / count as f64;
    let variance_rg = (rg_sq_sum / count as f64) - (mean_rg * mean_rg);
    let variance_yb = (yb_sq_sum / count as f64) - (mean_yb * mean_yb);
    let std_rg = variance_rg.sqrt().max(0.0);
    let std_yb = variance_yb.sqrt().max(0.0);

    (std_rg.powi(2) + std_yb.powi(2)).sqrt() + 0.3 * (mean_rg.powi(2) + mean_yb.powi(2)).sqrt()
}

fn detect_variant(colourfulness: f64) -> SchemeTypes {
    match colourfulness {
        ..15.0 => SchemeTypes::SchemeMonochrome,
        15.0..35.0 => SchemeTypes::SchemeContent,
        35.0..55.0 => SchemeTypes::SchemeTonalSpot,
        _ => SchemeTypes::SchemeVibrant,
    }
}

fn detect_mode(image: &DynamicImage) -> SchemesEnum {
    let resized = image.resize_exact(1, 1, FilterType::Lanczos3);
    let pixel = resized.get_pixel(0, 0);

    let rgb = Rgb::from((pixel[0] as f64, pixel[1] as f64, pixel[2] as f64));
    let argb = argb_from_rgb(&rgb);
    let hct: Hct = argb.into();

    if hct.get_tone() > 60.0 {
        SchemesEnum::Light
    } else {
        SchemesEnum::Dark
    }
}

fn get_cache_dir() -> Option<std::path::PathBuf> {
    get_proj_path(&ProjectDirsTypes::Cache).map(|p| p.join("smart"))
}

fn hash_file(path: &Path) -> Result<String, Report> {
    use sha2::{Digest, Sha256};

    let data = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(format!("{:x}", hasher.finalize()))
}

fn load_cache(hash: &str) -> Option<SmartOpts> {
    let cache_dir = get_cache_dir()?;
    let path = cache_dir.join(format!("{hash}.json"));

    let content = read_to_string(&path).ok()?;
    let cached: SmartCache = serde_json::from_str(&content).ok()?;

    let mode = match cached.mode.as_str() {
        "light" => SchemesEnum::Light,
        _ => SchemesEnum::Dark,
    };

    let variant = match cached.variant.as_str() {
        "scheme-monochrome" => SchemeTypes::SchemeMonochrome,
        "scheme-content" => SchemeTypes::SchemeContent,
        "scheme-vibrant" => SchemeTypes::SchemeVibrant,
        _ => SchemeTypes::SchemeTonalSpot,
    };

    Some(SmartOpts { mode, variant })
}

fn save_cache(hash: &str, opts: &SmartOpts) -> Result<(), Report> {
    let cache_dir =
        get_cache_dir().ok_or_else(|| Report::msg("Could not determine cache directory"))?;
    create_dir_all(&cache_dir)?;

    let path = cache_dir.join(format!("{hash}.json"));

    let variant_str = match opts.variant {
        SchemeTypes::SchemeMonochrome => "scheme-monochrome",
        SchemeTypes::SchemeContent => "scheme-content",
        SchemeTypes::SchemeTonalSpot => "scheme-tonal-spot",
        SchemeTypes::SchemeVibrant => "scheme-vibrant",
        _ => "scheme-tonal-spot",
    };

    let cached = SmartCache {
        mode: opts.mode.to_string(),
        variant: format!("{:?}", opts.variant).to_lowercase(),
    };

    let file = File::create(&path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(serde_json::to_string_pretty(&cached)?.as_bytes())?;

    Ok(())
}

pub fn get_smart_opts(image_path: &Path) -> Result<SmartOpts, Report> {
    let hash = hash_file(image_path)?;

    if let Some(cached) = load_cache(&hash) {
        return Ok(cached);
    }

    let img = ImageReader::open(image_path)?.decode()?;
    let thumb = img.thumbnail(128, 128);

    let mode = detect_mode(&thumb);
    let colourfulness = calc_colourfulness(&thumb);
    let variant = detect_variant(colourfulness);

    let opts = SmartOpts { mode, variant };

    save_cache(&hash, &opts)?;

    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_colourfulness_grayscale() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            128,
            128,
            image::Rgb([128, 128, 128]),
        ));
        let score = calc_colourfulness(&img);
        assert!(
            score < 1.0,
            "Grayscale image should have near-zero colourfulness, got {score}"
        );
    }

    #[test]
    fn test_colourfulness_solid_color() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            128,
            128,
            image::Rgb([128, 128, 128]),
        ));
        let score = calc_colourfulness(&img);
        assert!(
            score < 1.0,
            "Solid gray should have near-zero colourfulness, got {score}"
        );
    }

    #[test]
    fn test_colourfulness_colorful() {
        let mut img_buf = image::RgbImage::new(128, 128);
        for y in 0..128 {
            for x in 0..128 {
                let r = ((x * 2) % 256) as u8;
                let g = ((y * 2) % 256) as u8;
                let b = (((x + y) * 2) % 256) as u8;
                img_buf.put_pixel(x, y, image::Rgb([r, g, b]));
            }
        }
        let img = DynamicImage::ImageRgb8(img_buf);
        let score = calc_colourfulness(&img);
        assert!(
            score > 20.0,
            "Colorful gradient should score high, got {score}"
        );
    }

    #[test]
    fn test_detect_variant_boundaries() {
        assert!(matches!(detect_variant(0.0), SchemeTypes::SchemeMonochrome));
        assert!(matches!(detect_variant(15.0), SchemeTypes::SchemeContent));
        assert!(matches!(detect_variant(35.0), SchemeTypes::SchemeTonalSpot));
        assert!(matches!(detect_variant(55.0), SchemeTypes::SchemeVibrant));
        assert!(matches!(detect_variant(100.0), SchemeTypes::SchemeVibrant));
    }

    #[test]
    fn test_detect_mode_dark() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            128,
            128,
            image::Rgb([20, 20, 30]),
        ));
        assert!(matches!(detect_mode(&img), SchemesEnum::Dark));
    }

    #[test]
    fn test_detect_mode_light() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            128,
            128,
            image::Rgb([240, 240, 250]),
        ));
        assert!(matches!(detect_mode(&img), SchemesEnum::Light));
    }
}
