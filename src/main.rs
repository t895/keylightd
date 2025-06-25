use argh::FromArgs;
use command::{GetKeyboardBacklight, SetKeyboardBacklight};
use ec::EmbeddedController;
use evdev::Device;
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};
use std::os::fd::AsRawFd;
use std::{io, thread, time::Duration};

use crate::command::{LedBrightnesses, LedControl, LedFlags, LedId};

mod command;
mod ec;

/// keylightd - automatic keyboard backlight daemon for Framework laptops
#[derive(Debug, FromArgs)]
struct Args {
    /// brightness level when active (0-100) [default=30]
    #[argh(option, default = "30", from_str_fn(parse_brightness))]
    brightness: u8,

    /// activity timeout in seconds [default=10]
    #[argh(option, default = "10")]
    timeout: u32,

    /// also control the power LED in the fingerprint module
    #[argh(switch)]
    power: bool,
}

fn parse_brightness(s: &str) -> Result<u8, String> {
    let brightness = s.parse::<u8>().map_err(|e| e.to_string())?;
    if brightness > 100 {
        return Err("invalid brightness value {brightness} (valid range: 0-100)".into());
    }
    Ok(brightness)
}

fn fade_to(ec: &EmbeddedController, power: bool, target: u8) -> io::Result<()> {
    let resp = ec.command(GetKeyboardBacklight)?;
    let mut cur = if resp.enabled != 0 { resp.percent } else { 0 };
    while cur != target {
        if cur > target {
            cur -= 1;
        } else {
            cur += 1;
        }

        if power {
            // The power LED cannot be faded from software (although the beta BIOS apparently
            // has a switch for dimming it, so maybe it'll work with the next BIOS update).
            // So instead, we treat 0 as off and set it back to auto for any non-zero value.
            if cur == 0 {
                ec.command(LedControl {
                    led_id: LedId::POWER,
                    flags: LedFlags::NONE,
                    brightness: LedBrightnesses::default(),
                })?;
            } else if cur == 1 {
                ec.command(LedControl {
                    led_id: LedId::POWER,
                    flags: LedFlags::AUTO,
                    brightness: LedBrightnesses::default(),
                })?;
            }
        }

        ec.command(SetKeyboardBacklight { percent: cur })?;

        thread::sleep(Duration::from_millis(3));
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::builder()
        .filter_module(
            env!("CARGO_PKG_NAME"),
            if cfg!(debug_assertions) {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            },
        )
        .init();

    let args: Args = argh::from_env();
    log::debug!("args={:?}", args);

    let mut devices = Vec::<Device>::new();
    for (_, device) in evdev::enumerate() {
        // Filter devices so that only the Framework's builtin touchpad and keyboard are listened
        // to. Since we don't support hotplug, listening on USB devices wouldn't work reliably.
        match device.name() {
            Some("PIXA3854:00 093A:0274 Touchpad" | "AT Translated Set 2 keyboard") => {
                log::info!(
                    "Got device - {} - {:?}",
                    device.name().unwrap(),
                    device.input_id()
                );
                devices.push(device);
            }
            _ => {}
        }
    }

    log::info!("idle timeout: {} seconds", args.timeout);
    log::info!("brightness level: {}%", args.brightness);

    let mut poller = Poll::new()?;
    for device in &devices {
        poller.registry().register(
            &mut SourceFd(&device.as_raw_fd()),
            Token(device.input_id().product() as usize),
            Interest::READABLE,
        )?;
    }
    let mut events = Events::with_capacity(1);

    let mut active = false;
    let timeout = Duration::from_secs(args.timeout.into());

    let ec = EmbeddedController::open()?;
    loop {
        poller.poll(&mut events, Some(timeout))?;

        if events.is_empty() {
            if active {
                fade_to(&ec, args.power, 0)?;
                active = false;
            }
        } else {
            if !active {
                fade_to(&ec, args.power, args.brightness)?;
                active = true;
            }

            // Limit the rate of fade-in updates.
            thread::sleep(Duration::from_millis(500));
        }
    }
}
