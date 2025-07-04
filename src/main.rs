use argh::FromArgs;
use command::{GetKeyboardBacklight, SetKeyboardBacklight};
use ec::EmbeddedController;
use mio::{Events, Interest, Poll, Token};
use std::{io, thread, time::Duration};

use crate::command::{LedBrightnesses, LedControl, LedFlags, LedId};

mod command;
mod ec;

/// keylightd - automatic keyboard backlight daemon for Framework laptops
#[derive(Debug, FromArgs)]
struct Args {
    /// activity timeout in seconds [default=20]
    #[argh(option, default = "20")]
    timeout: u32,

    /// also control the power LED in the fingerprint module
    #[argh(switch)]
    power: bool,
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

#[cfg(unix)]
fn register_devices(poller: &Poll, devices: &mut Vec<evdev::Device>) -> io::Result<()> {
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

                poller.registry().register(
                    &mut mio::unix::SourceFd(&std::os::fd::AsRawFd::as_raw_fd(&device)),
                    Token(device.input_id().product() as usize),
                    Interest::READABLE,
                )?;
                devices.push(device);
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(windows)]
fn register_devices(poller: &Poll, devices: &mut Vec<u8>) -> io::Result<()> {
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

    let mut poller = Poll::new()?;
    let mut devices = Vec::new();
    register_devices(&poller, &mut devices)?;

    log::info!("idle timeout: {} seconds", args.timeout);

    let timeout = Duration::from_secs(args.timeout.into());

    let ec = EmbeddedController::open()?;
    let mut max_brightness = ec.command(GetKeyboardBacklight)?.percent;
    let mut active = max_brightness > 0;

    let mut events = Events::with_capacity(1);
    loop {
        poller.poll(
            &mut events,
            if active { Some(timeout) } else { None }
        )?;

        if active {
            max_brightness = ec.command(GetKeyboardBacklight)?.percent;
        }

        if events.is_empty() {
            if active {
                fade_to(&ec, args.power, 0)?;
                active = false;
            }
        } else {
            if !active {
                fade_to(&ec, args.power, max_brightness)?;
                active = true;
            }

            // Limit the rate of fade-in updates.
            thread::sleep(Duration::from_millis(500));
        }
    }
}
