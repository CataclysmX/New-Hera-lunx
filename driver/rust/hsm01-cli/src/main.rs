//! `hsm01` – command-line tool for the Hercules Stream 100.

mod fw;
mod logo;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hsm01_proto::cmd::{Bank, Command, LedState};
use hsm01_proto::input::{Controls, DeviceMessage, buttons, encoder_delta};
use hsm01_proto::rle::{self, rgb};
use hsm01_proto::{CHANNELS, ICON_H, ICON_W, LABEL_H, LABEL_W, LEDS};
use hsm01_usb::{Device, Result};

const USAGE: &str = "usage: hsm01 <command>
  info                 firmware, serial and device-info message
  monitor [secs]       print button / knob / screen events (default 30 s)
  demo [secs]          test pattern: icons, labels, LEDs, animated VU meters (default 20 s)
  brightness <0-100>   set the screen backlight
  led <0-3> <off|on|blink>
  background <image> [secs]   show an image as full-screen background (RAM only, default 15 s)
  splash <image> --yes        write the boot logo to flash (480x272, any PNG/JPEG/BMP/GIF)
  splash default --yes        restore the factory boot logo
custom firmware (firmware/rust, reports version 240.xx):
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs = |i: usize, d: u64| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let yes = args.iter().any(|a| a == "--yes");
    let res: std::result::Result<(), Box<dyn std::error::Error>> =
        match args.first().map(String::as_str) {
            Some("fw") => fw::run(&args[1..]),
            Some("info") => info().map_err(Into::into),
            Some("monitor") => monitor(secs(1, 30)).map_err(Into::into),
            Some("demo") => demo(secs(1, 20)).map_err(Into::into),
            Some("background") => match args.get(1) {
                Some(p) => logo::background(p, secs(2, 15)),
                None => return usage(),
            },
            Some("splash") => match args.get(1).filter(|a| *a != "--yes") {
                Some(t) => logo::splash(t, yes),
                None => return usage(),
            },
            Some("brightness") => match args.get(1).and_then(|s| s.parse::<u8>().ok()) {
                Some(p) => {
                    one_shot(&[Command::ScreenOn, Command::ScreenBrightness(p)]).map_err(Into::into)
                }
                None => return usage(),
            },
            Some("led") => match (
                args.get(1).and_then(|s| s.parse::<u8>().ok()),
                args.get(2).map(String::as_str),
            ) {
                (Some(i), Some(s)) if (i as usize) < LEDS => {
                    let state = match s {
                        "off" => LedState::Off,
                        "on" => LedState::On,
                        "blink" => LedState::Blink,
                        _ => return usage(),
                    };
                    one_shot(&[Command::Led { index: i, state }]).map_err(Into::into)
                }
                _ => return usage(),
            },
            _ => return usage(),
        };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hsm01: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}{}", fw::USAGE);
    ExitCode::from(2)
}

fn open() -> Result<Arc<Device>> {
    Ok(Arc::new(Device::open()?))
}

fn one_shot(cmds: &[Command]) -> Result<()> {
    let dev = open()?;
    dev.keepalive()?;
    dev.send(cmds)
}

fn info() -> Result<()> {
    let dev = open()?;
    let (maj, min) = dev.firmware_version()?;
    let sn = dev.serial()?;
    println!("firmware {maj}.{min:02}  serial {}", hex(&sn));
    let _ka = dev.spawn_keepalive();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Some(r) = dev.poll(Duration::from_millis(100))? {
            match r.message {
                Some(DeviceMessage::Info(i)) => {
                    println!(
                        "type {} fw {}.{:02} screen {}x{} icon {}x{} label {}x{} buttons {} leds {}",
                        i.device_type,
                        i.firmware.0,
                        i.firmware.1,
                        i.screen_w,
                        i.screen_h,
                        i.icon_size.0,
                        i.icon_size.1,
                        i.label_size.0,
                        i.label_size.1,
                        i.button_count,
                        i.led_count
                    );
                    return Ok(());
                }
                Some(DeviceMessage::Status(s)) if s.error != 0 => {
                    println!("status error {:#06x}", s.error)
                }
                _ => {}
            }
        }
    }
    println!("no device-info message received");
    Ok(())
}

fn monitor(secs: u64) -> Result<()> {
    let dev = open()?;
    let _ka = dev.spawn_keepalive();
    let end = Instant::now() + Duration::from_secs(secs);
    let mut prev: Option<Controls> = None;
    println!("monitoring for {secs} s (Ctrl+C to stop)");
    while Instant::now() < end {
        let Some(r) = dev.poll(Duration::from_millis(100))? else {
            continue;
        };
        if let Some(DeviceMessage::Status(s)) = &r.message
            && (s.error != 0 || s.last_invalid_cmd != 0)
        {
            println!(
                "device error {:#06x} last invalid cmd {:#04x}",
                s.error, s.last_invalid_cmd
            );
        }
        let Some(c) = r.controls else { continue };
        if let Some(p) = prev {
            let changed = c.buttons ^ p.buttons;
            for (bit, name) in buttons::NAMES.iter().enumerate() {
                if changed & (1 << bit) != 0 {
                    println!(
                        "{name} {}",
                        if c.buttons & (1 << bit) != 0 {
                            "pressed"
                        } else {
                            "released"
                        }
                    );
                }
            }
            for k in 0..4 {
                let d = encoder_delta(p.encoders[k], c.encoders[k]);
                if d != 0 {
                    println!("knob{} {d:+}", k + 1);
                }
            }
            if (c.screen_on, c.brightness) != (p.screen_on, p.brightness) {
                println!(
                    "screen {} {}%",
                    if c.screen_on { "on" } else { "off" },
                    c.brightness
                );
            }
        }
        prev = Some(c);
    }
    Ok(())
}

/// Solid disc of `color` on a transparent 32×32 icon.
fn icon(color: u32) -> Vec<u32> {
    let mut px = vec![0u32; ICON_W * ICON_H];
    for y in 0..ICON_H {
        for x in 0..ICON_W {
            let (dx, dy) = (x as i32 * 2 - 31, y as i32 * 2 - 31);
            if dx * dx + dy * dy <= 30 * 30 {
                px[y * ICON_W + x] = color;
            }
        }
    }
    px
}

/// Horizontal gradient bar with `n` white tally marks (channel number) on a 110×16 label.
fn label(color: u32, n: usize) -> Vec<u32> {
    let mut px = vec![0u32; LABEL_W * LABEL_H];
    for y in 2..LABEL_H - 2 {
        for x in 0..LABEL_W {
            let k = 64 + (x * 191 / LABEL_W) as u32;
            let (r, g, b) = ((color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff);
            px[y * LABEL_W + x] = rgb(
                (r * k / 255) as u8,
                (g * k / 255) as u8,
                (b * k / 255) as u8,
            );
        }
    }
    for i in 0..n {
        for y in 4..LABEL_H - 4 {
            for x in 0..3 {
                px[y * LABEL_W + 8 + i * 8 + x] = rgb(255, 255, 255);
            }
        }
    }
    px
}

fn demo(secs: u64) -> Result<()> {
    const COLORS: [u32; CHANNELS] = [
        rgb(255, 64, 64),
        rgb(64, 255, 64),
        rgb(64, 128, 255),
        rgb(255, 200, 0),
    ];
    let dev = open()?;
    let _ka = dev.spawn_keepalive();
    // Same configuration as the official app at start-up (captured).
    let mut init = vec![
        Command::ScreenOn,
        Command::ScreenBrightness(100),
        Command::LedIntensity(90),
        Command::ChannelDisplay([[0x81, 0x01, 0x07, 0x07, 0x00, 0x64]; 4]),
        Command::ScreenConfig {
            screen_type: 0,
            background_type: 4,
            darkest_rgb: 0xffff_ffff,
            lightest_rgb: 0xffff_ffff,
            action_zone_style: 4,
            action_zone_rgb: 0xffff_ffff,
        },
    ];
    for ch in 0..CHANNELS as u8 {
        for sub in 0..2u8 {
            let grad = if sub == 0 { 0x2945 } else { 0x8410 };
            init.push(Command::VuStyle {
                channel: ch,
                sub,
                colors: [0x2104, 0xf800, 0xffff],
                gradient: vec![grad; 2],
            });
        }
    }
    dev.send(&init)?;
    for (ch, &color) in COLORS.iter().enumerate() {
        let ic = rle::encode(&icon(color), ICON_W, ICON_H);
        let lb = rle::encode(&label(color, ch + 1), LABEL_W, LABEL_H);
        for bank in [Bank::A, Bank::B] {
            dev.send(&[
                Command::Icon {
                    bank,
                    channel: ch as u8,
                    rle: ic.clone(),
                },
                Command::Label {
                    bank,
                    channel: ch as u8,
                    rle: lb.clone(),
                },
            ])?;
        }
    }
    println!(
        "demo running for {secs} s: icons/labels on 4 channels, LED chaser, animated VU meters"
    );
    let t0 = Instant::now();
    let mut errors = 0u32;
    while t0.elapsed() < Duration::from_secs(secs) {
        let t = t0.elapsed().as_secs_f32();
        let mut cmds = Vec::new();
        let lit = (t * 4.0) as usize % LEDS;
        for i in 0..LEDS {
            cmds.push(Command::Led {
                index: i as u8,
                state: if i == lit {
                    LedState::On
                } else {
                    LedState::Off
                },
            });
        }
        for ch in 0..CHANNELS as u8 {
            for sub in 0..2u8 {
                let phase = t * (1.5 + ch as f32 * 0.4) + sub as f32;
                let v = (phase.sin() * 0.5 + 0.5) * 0xffff as f32;
                cmds.push(Command::VuLevel {
                    channel: ch,
                    sub,
                    level: v as u16,
                    peak: v as u16,
                    clip: v > 62000.0,
                });
            }
        }
        dev.send(&cmds)?;
        while let Some(r) = dev.poll(Duration::from_millis(1))? {
            if let Some(DeviceMessage::Status(s)) = r.message
                && (s.error != 0 || s.last_invalid_cmd != 0)
            {
                errors += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    println!("done, status reports with errors: {errors}");
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
