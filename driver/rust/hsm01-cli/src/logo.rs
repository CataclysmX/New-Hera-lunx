//! `background` / `splash` sub-commands: custom full-screen background and boot logo.

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hsm01_proto::background::{IndexedImage, quantize};
use hsm01_proto::cmd::Command;
use hsm01_proto::input::DeviceMessage;
use hsm01_proto::{SCREEN_H, SCREEN_W};
use hsm01_usb::Device;

type AnyResult<T> = Result<T, Box<dyn Error>>;

/// Load any PNG/JPEG/BMP/GIF, scale + center-crop to 480×272, return ARGB8888 pixels.
pub fn load_image(path: &str) -> AnyResult<Vec<u32>> {
    let img = image::open(path)?
        .resize_to_fill(
            SCREEN_W as u32,
            SCREEN_H as u32,
            image::imageops::FilterType::Lanczos3,
        )
        .to_rgb8();
    Ok(img
        .pixels()
        .map(|p| 0xff00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
        .collect())
}

/// Show the image as custom background (RAM only) by sending palette + 32 tiles.
fn upload(dev: &Device, img: &IndexedImage) -> AnyResult<()> {
    dev.send(&[
        Command::ScreenOn,
        // host mode starts with the backlight at 0: without this the screen looks off (live test)
        Command::ScreenBrightness(100),
        Command::ScreenConfig {
            screen_type: 0,
            background_type: 0xff, // custom image
            darkest_rgb: 0,
            lightest_rgb: 0,
            action_zone_style: 0,
            action_zone_rgb: 0,
        },
    ])?;
    let (palette, tiles) = img.commands();
    for (i, tile) in tiles.into_iter().enumerate() {
        let cmds = if i == 0 {
            vec![palette.clone(), tile]
        } else {
            vec![tile]
        };
        dev.send(&cmds)?;
        // the SDK sends one tile every other frame: give the firmware time to blit it
        let until = Instant::now() + Duration::from_millis(40);
        while Instant::now() < until {
            if let Some(r) = dev.poll(Duration::from_millis(5))?
                && let Some(DeviceMessage::Status(s)) = r.message
                && (s.error != 0 || s.last_invalid_cmd != 0)
            {
                return Err(format!("device rejected tile {i}: error {:#06x}", s.error).into());
            }
        }
    }
    println!("background uploaded ({} colors)", img.palette.len());
    Ok(())
}

pub fn background(path: &str, secs: u64) -> AnyResult<()> {
    let img = quantize(&load_image(path)?);
    let dev = Arc::new(Device::open()?);
    let _ka = dev.spawn_keepalive();
    upload(&dev, &img)?;
    println!("preview for {secs} s (RAM only, nothing written to flash)");
    // Keep reading reports (poll() acknowledges device-info messages): a host that only sends
    // keep-alives is dropped by the official firmware after a few seconds (Hercules logo, then
    // screen off – live test 2026-11-20).
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        dev.poll(Duration::from_millis(100))?;
    }
    Ok(())
}

/// Wait for the flash write to finish (status byte: 0..=100 progress, 0xfe error, 0xff idle).
fn wait_flash(dev: &Device) -> AnyResult<()> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let (mut last, mut started) = (u8::MAX, false);
    while Instant::now() < deadline {
        let Some(r) = dev.poll(Duration::from_millis(100))? else {
            continue;
        };
        let Some(DeviceMessage::Status(s)) = r.message else {
            continue;
        };
        if s.error & 0x100 != 0 {
            return Err("device refused the flash write (incomplete background?)".into());
        }
        match s.flash_status {
            0xfe => return Err("flash write failed (device status 0xfe)".into()),
            p @ 0..=100 => {
                started = true;
                if p != last {
                    println!("flash write {p}%");
                    last = p;
                }
                if p == 100 {
                    return Ok(());
                }
            }
            _ if started => return Ok(()), // back to idle after completion
            _ => {}
        }
    }
    Err("timeout waiting for the flash write".into())
}

/// `splash <image|default> --yes`
pub fn splash(target: &str, yes: bool) -> AnyResult<()> {
    if !yes {
        return Err(
            "this writes the boot logo to the device flash; re-run with --yes \
                    (`hsm01 splash default --yes` restores the factory logo)"
                .into(),
        );
    }
    let dev = Arc::new(Device::open()?);
    let _ka = dev.spawn_keepalive();
    if target == "default" {
        dev.send(&[Command::SplashFlash { custom: false }])?;
    } else {
        let img = quantize(&load_image(target)?);
        upload(&dev, &img)?;
        dev.send(&[Command::SplashFlash { custom: true }])?;
    }
    wait_flash(&dev)?;
    println!("boot logo updated – unplug/replug the device to see it");
    Ok(())
}
