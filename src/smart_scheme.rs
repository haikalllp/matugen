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
        15.0..30.0 => SchemeTypes::SchemeNeutral,
        30.0..65.0 => SchemeTypes::SchemeTonalSpot,
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

fn load_cache_from_dir(hash: &str, cache_dir: &Path) -> Option<SmartOpts> {
    let path = cache_dir.join(format!("{hash}.json"));

    let content = read_to_string(&path).ok()?;
    let cached: SmartCache = serde_json::from_str(&content).ok()?;

    let mode = match cached.mode.as_str() {
        "light" => SchemesEnum::Light,
        _ => SchemesEnum::Dark,
    };

    let variant = match cached.variant.as_str() {
        "scheme-monochrome" => SchemeTypes::SchemeMonochrome,
        "scheme-neutral" => SchemeTypes::SchemeNeutral,
        "scheme-vibrant" => SchemeTypes::SchemeVibrant,
        _ => SchemeTypes::SchemeTonalSpot,
    };

    success!("Loaded smart cache from <d><u>{}</>", path.display());

    Some(SmartOpts { mode, variant })
}

fn load_cache(hash: &str) -> Option<SmartOpts> {
    let cache_dir = get_cache_dir()?;
    load_cache_from_dir(hash, &cache_dir)
}

fn save_cache_to_dir(hash: &str, opts: &SmartOpts, cache_dir: &Path) -> Result<(), Report> {
    create_dir_all(cache_dir)?;

    let path = cache_dir.join(format!("{hash}.json"));

    let variant_str = match opts.variant {
        SchemeTypes::SchemeMonochrome => "scheme-monochrome",
        SchemeTypes::SchemeNeutral => "scheme-neutral",
        SchemeTypes::SchemeTonalSpot => "scheme-tonal-spot",
        SchemeTypes::SchemeVibrant => "scheme-vibrant",
        _ => "scheme-tonal-spot",
    };

    let cached = SmartCache {
        mode: opts.mode.to_string(),
        variant: variant_str.to_string(),
    };

    let file = File::create(&path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(serde_json::to_string_pretty(&cached)?.as_bytes())?;

    success!("Saved smart cache to <d><u>{}</>", path.display());

    Ok(())
}

fn save_cache(hash: &str, opts: &SmartOpts) -> Result<(), Report> {
    let cache_dir =
        get_cache_dir().ok_or_else(|| Report::msg("Could not determine cache directory"))?;
    save_cache_to_dir(hash, opts, &cache_dir)
}

pub fn get_smart_opts(image_path: &Path, use_cache: bool) -> Result<SmartOpts, Report> {
    let hash = if use_cache {
        hash_file(image_path).ok()
    } else {
        None
    };

    if let Some(hash) = hash.as_ref() {
        if let Some(cached) = load_cache(hash) {
            return Ok(cached);
        }
    }

    let img = ImageReader::open(image_path)?.decode()?;
    let thumb = img.thumbnail(128, 128);

    let mode = detect_mode(&thumb);
    let colourfulness = calc_colourfulness(&thumb);
    let variant = detect_variant(colourfulness);

    let opts = SmartOpts { mode, variant };

    if let Some(hash) = hash.as_ref() {
        save_cache(hash, &opts)?;
    }

    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_image(path: &Path, dark: bool) {
        let color = if dark {
            image::Rgb([20, 20, 30])
        } else {
            image::Rgb([240, 240, 250])
        };
        let img = image::RgbImage::from_pixel(64, 64, color);
        img.save(path).unwrap();
    }

    fn create_unique_test_image(path: &Path, seed: u8) {
        let mut img = image::RgbImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let r = ((x as u16 + seed as u16) % 256) as u8;
                let g = ((y as u16 + seed as u16 * 3) % 256) as u8;
                let b = seed;
                img.put_pixel(x, y, image::Rgb([r, g, b]));
            }
        }
        img.save(path).unwrap();
    }

    #[test]
    fn test_hash_file_consistency() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test.png");
        create_test_image(&img_path, true);

        let hash1 = hash_file(&img_path).unwrap();
        let hash2 = hash_file(&img_path).unwrap();
        assert_eq!(hash1, hash2, "Same file should produce same hash");
    }

    #[test]
    fn test_hash_file_different_images() {
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.png");
        let path_b = dir.path().join("b.png");
        create_test_image(&path_a, true);
        create_test_image(&path_b, false);

        let hash_a = hash_file(&path_a).unwrap();
        let hash_b = hash_file(&path_b).unwrap();
        assert_ne!(hash_a, hash_b, "Different images should produce different hashes");
    }

    #[test]
    fn test_save_and_load_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("smart");
        create_dir_all(&cache_dir).unwrap();

        let hash = "testhash123";
        let opts = SmartOpts {
            mode: SchemesEnum::Light,
            variant: SchemeTypes::SchemeVibrant,
        };

        save_cache_to_dir(hash, &opts, &cache_dir).unwrap();

        let loaded = load_cache_from_dir(hash, &cache_dir).unwrap();
        assert!(matches!(loaded.mode, SchemesEnum::Light));
        assert!(matches!(loaded.variant, SchemeTypes::SchemeVibrant));
    }

    #[test]
    fn test_smart_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test_roundtrip.png");
        create_unique_test_image(&img_path, 42);

        let result1 = get_smart_opts(&img_path, true).unwrap();
        let result2 = get_smart_opts(&img_path, true).unwrap();

        assert_eq!(result1.mode, result2.mode);
        assert_eq!(result1.variant, result2.variant);
    }

    #[test]
    fn test_smart_cache_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test_disabled.png");
        create_unique_test_image(&img_path, 99);

        let hash = hash_file(&img_path).unwrap();
        let cache_dir = get_cache_dir().unwrap_or_else(|| std::env::temp_dir().join("matugen").join("smart"));
        let cache_path = cache_dir.join(format!("{hash}.json"));

        let _ = fs::remove_file(&cache_path);

        let _result = get_smart_opts(&img_path, false).unwrap();
        assert!(!cache_path.exists(), "Cache file should NOT be created when caching is disabled");
    }

    #[test]
    fn test_smart_cache_actually_hits_cache() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test_hits_cache.png");
        create_unique_test_image(&img_path, 77);

        let hash = hash_file(&img_path).unwrap();

        let cache_dir = get_cache_dir().unwrap_or_else(|| std::env::temp_dir().join("matugen").join("smart"));
        let cache_path = cache_dir.join(format!("{hash}.json"));
        let _ = fs::remove_file(&cache_path);

        let result1 = get_smart_opts(&img_path, true).unwrap();
        assert!(cache_path.exists(), "Cache file should exist after first call");

        let metadata_before = fs::metadata(&cache_path).unwrap();
        let modified_before = metadata_before.modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let result2 = get_smart_opts(&img_path, true).unwrap();

        let metadata_after = fs::metadata(&cache_path).unwrap();
        let modified_after = metadata_after.modified().unwrap();

        assert_eq!(modified_before, modified_after, "Cache file should NOT be rewritten on hit");

        assert_eq!(result1.mode, result2.mode);
        assert_eq!(result1.variant, result2.variant);
    }

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
        assert!(matches!(detect_variant(15.0), SchemeTypes::SchemeNeutral));
        assert!(matches!(detect_variant(30.0), SchemeTypes::SchemeTonalSpot));
        assert!(matches!(detect_variant(65.0), SchemeTypes::SchemeVibrant));
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
