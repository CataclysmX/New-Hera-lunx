//! `hsm01 fw …` – commands for the custom firmware ("free screen" protocol).

use std::error::Error;
use std::time::{Duration, Instant};

use hsm01_proto::free::{self, rgb565};
use hsm01_proto::input::buttons;
use hsm01_proto::{SCREEN_H, SCREEN_W};
use hsm01_usb::free::FreeDevice;

type AnyResult<T> = Result<T, Box<dyn Error>>;

pub const USAGE: &str = "  fw info                    custom firmware version
  fw fill <r> <g> <b>        fill the whole screen
  fw image <file>            show a PNG/JPEG/BMP/GIF (scaled + cropped to 480x272)
  fw backlight <0-100>       screen backlight
  fw leds <a> <b> <c> <d>    LED intensities 0-100
  fw monitor [secs]          buttons + raw knob wipers
  fw bench [frames]          full-screen update throughput
  fw dfu                     reboot into the ST DFU bootloader";

pub fn run(args: &[String]) -> AnyResult<()> {
    let num =
        |i: usize| -> AnyResult<u8> { Ok(args.get(i).ok_or("missing argument")?.parse::<u8>()?) };
    let dev = FreeDevice::open()?;
    let mut out = Vec::new();
    match args.first().map(String::as_str) {
        Some("info") => {
            let (maj, min) = dev.version();
            println!("custom firmware {maj}.{min:02} (stage {min})");
        }
        Some("fill") => {
            free::fill(
                0,
                0,
                SCREEN_W as u16,
                SCREEN_H as u16,
                rgb565(num(1)?, num(2)?, num(3)?),
                &mut out,
            );
            dev.send(&out)?;
            hold(&dev)?;
        }
        Some("image") => {
            let argb = crate::logo::load_image(args.get(1).ok_or("missing image path")?)?;
            let px: Vec<u16> = argb
                .iter()
                .map(|p| rgb565((p >> 16) as u8, (p >> 8) as u8, *p as u8))
                .collect();
            free::blit_auto(0, 0, SCREEN_W as u16, SCREEN_H as u16, &px, &mut out);
            let t = Instant::now();
            dev.send(&out)?;
            println!(
                "sent {} KiB in {:.0} ms",
                out.len() / 1024,
                t.elapsed().as_secs_f64() * 1e3
            );
            hold(&dev)?;
        }
        Some("backlight") => {
            free::backlight(num(1)?, &mut out);
            dev.send(&out)?;
            hold(&dev)?;
        }
        Some("leds") => {
            free::leds([num(1)?, num(2)?, num(3)?, num(4)?], &mut out);
            dev.send(&out)?;
            hold(&dev)?;
        }
        Some("monitor") => monitor(&dev, args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30))?,
        Some("bench") => bench(&dev, args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30))?,
        Some("dfu") => {
            dev.reboot_to_dfu()?;
            println!("rebooting into DFU (0483:df11)");
        }
        _ => return Err(format!("usage:\n{USAGE}").into()),
    }
    Ok(())
}

/// From 240.22 the firmware takes the screen back 3 s after its host stops reading the input
/// reports (screen saver): read them until Ctrl+C so that what was sent stays.
fn hold(dev: &FreeDevice) -> AnyResult<()> {
    if dev.version().1 < free::SAVER_SINCE {
        return Ok(());
    }
    println!("kept on the device until Ctrl+C");
    loop {
        dev.read_input(Duration::from_millis(500))?;
    }
}

fn monitor(dev: &FreeDevice, secs: u64) -> AnyResult<()> {
    println!(
        "monitoring {secs} s – knobs in official-firmware counts ({} per cycle)",
        free::knob::COUNTS_PER_CYCLE
    );
    let end = Instant::now() + Duration::from_secs(secs);
    let mut last_btn = u16::MAX;
    let mut dec = [free::knob::Decoder::default(); 4];
    let (mut pending, mut total, mut last_print) = ([0i32; 4], [0i32; 4], Instant::now());
    while Instant::now() < end {
        let Some(r) = dev.read_input(Duration::from_millis(100))? else {
            continue;
        };
        if r.buttons != last_btn {
            let names: Vec<&str> = (0..9)
                .filter(|b| r.buttons & (1 << b) != 0)
                .map(|b| buttons::NAMES[b])
                .collect();
            println!("buttons {:#06x} {:?}", r.buttons, names);
            last_btn = r.buttons;
        }
        for k in 0..4 {
            pending[k] += dec[k].update(r.wipers[2 * k], r.wipers[2 * k + 1]);
        }
        if last_print.elapsed() > Duration::from_millis(50) {
            for k in 0..4 {
                if pending[k] != 0 {
                    total[k] += pending[k];
                    println!("knob{} {:+} (total {:+})", k + 1, pending[k], total[k]);
                    pending[k] = 0;
                }
            }
            last_print = Instant::now();
        }
    }
    Ok(())
}

fn bench(dev: &FreeDevice, frames: u32) -> AnyResult<()> {
    let colors = [
        rgb565(255, 0, 0),
        rgb565(0, 255, 0),
        rgb565(0, 0, 255),
        rgb565(255, 255, 255),
    ];
    let t = Instant::now();
    let mut bytes = 0usize;
    for i in 0..frames {
        // raw full-screen blit: worst case (no compression)
        let px = vec![colors[i as usize % 4]; SCREEN_W * SCREEN_H];
        let mut out = Vec::new();
        free::blit(0, 0, SCREEN_W as u16, SCREEN_H as u16, &px, &mut out);
        dev.send(&out)?;
        bytes += out.len();
    }
    let s = t.elapsed().as_secs_f64();
    println!(
        "{frames} raw full-screen frames in {s:.2} s: {:.1} fps, {:.0} KiB/s",
        frames as f64 / s,
        bytes as f64 / 1024.0 / s
    );
    Ok(())
}
