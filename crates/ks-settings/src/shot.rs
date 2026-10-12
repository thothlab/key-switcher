//! Только для отладочной сборки: окно снимает свой кадр в BMP и закрывается —
//! `KS_SETTINGS_SHOT=<файл.bmp> [KS_SETTINGS_SIZE=640x420] keyswitcher --settings --section …`.
//! Так вид окна проверяется без права «Запись экрана» у терминала.

use std::path::PathBuf;

use eframe::egui::{self, ColorImage};

pub struct Shot {
    path: PathBuf,
    frames: u32,
}

impl Shot {
    pub fn from_env() -> Option<Shot> {
        let path = std::env::var_os("KS_SETTINGS_SHOT")?;
        Some(Shot {
            path: path.into(),
            frames: 0,
        })
    }

    /// Размер окна из `KS_SETTINGS_SIZE` («640x420»).
    pub fn size() -> Option<[f32; 2]> {
        let size = std::env::var("KS_SETTINGS_SIZE").ok()?;
        let (w, h) = size.split_once('x')?;
        Some([w.parse().ok()?, h.parse().ok()?])
    }

    /// Каждый кадр: на пятом просит снимок, получив — пишет файл и закрывает окно.
    pub fn frame(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        if self.frames == 5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            if let Err(e) = std::fs::write(&self.path, bmp(&image)) {
                eprintln!("снимок не записан: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

/// 32-битный BMP снизу вверх.
fn bmp(image: &ColorImage) -> Vec<u8> {
    let [w, h] = image.size;
    let data = w * h * 4;
    let mut out = Vec::with_capacity(54 + data);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(data as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for row in image.pixels.chunks(w).rev() {
        for p in row {
            out.extend_from_slice(&[p.b(), p.g(), p.r(), 255]);
        }
    }
    out
}
