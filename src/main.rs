#[macro_use]
extern crate log;

mod pid;
mod ipmi;
mod control;

use ipmi::*;
use control::*;

use env_logger;
use clap::{Arg, App};
use ctrlc;

use std::time::Instant;
use std::io::Result;

fn main() {
	env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();

	let matches = App::new("Thermal Watchdog")	
					.version("0.1")
					.author("Val Vanderschaegen <val@vvanders.com>")
					.about("Fan monitoring for IPMI based platforms. USE AT YOUR OWN RISK, NO WARRANTY IS EXPRESSED OR IMPLIED!")
					.arg(Arg::with_name("live")
						.long("live")
						.short("l")
						.help("Enables IPMI control, by default TWD operates in \"shadow\" mode. Logging outputs but taking no action"))
					.arg(Arg::with_name("influx_addr")
						.short("i")
						.long("influx_addr")
						.takes_value(true)
						.help("InfluxDB server address"))
					.arg(Arg::with_name("influx_user")
						.short("u")
						.long("influx_user")
						.takes_value(true)
						.help("InfluxDB user"))
					.arg(Arg::with_name("influx_pw")
						.short("p")
						.long("influx_ps")
						.takes_value(true)
						.help("InfluxDB password"))
		.get_matches();

	let shadow = !matches.is_present("live");

	ctrlc::set_handler(move || {
		info!("Signal received, aborting and resetting IPMI control");
		set_fan_manual(false, shadow).unwrap_or(());
		::std::process::exit(1);
	}).expect("Unable to set signal handler");

	main_loop(shadow);
}

fn main_loop(shadow: bool) {
	let mut control_loop = ControlLoop::new();
	
	let cpu_k = 0.05;
	let cpu_i = 0.000001;
	let cpu_d = 0.0;

	control_loop.add_control("Exhaust Temp".to_string(), 40.0, (cpu_k,cpu_i,cpu_d), 60.0);
	control_loop.add_control("Temp".to_string(), 60.0, (cpu_k,cpu_i,cpu_d), 65.0);
	control_loop.add_control("Temp".to_string(), 60.0, (cpu_k,cpu_i,cpu_d), 65.0);

	if shadow {
		info!("TWD running in Shadow Mode, no IPMI commands will be issued");
	}
	
	let mut manual = false;
	let mut last_update = Instant::now();
	loop {
		let now = Instant::now();
		let duration = now.duration_since(last_update);
		last_update = now;

		let elapsed = (duration.as_secs() * 1000 + duration.subsec_millis() as u64) as f32;
		let loop_result = control_loop.step(elapsed);

		let set_result = match loop_result {
			Ok(control) => {
				let enable = if !manual {
					info!("Enabling manual fan control");
					set_fan_manual(true, shadow)
						.and_then(|_| {
							manual = true;
							Ok(())
						})
				} else {
					Ok(())
				};

				enable.and_then(|_| set_fan_speed(control, shadow))
			},
			Err(e) => {
				error!("Unable to run control, resetting to manual: {}", e);
				set_fan_manual(false, shadow).and_then(|_| {
					manual = false;
					Ok(())
				})
			}
		};

		if let Err(_) = set_result {
			set_fan_manual(false, shadow).unwrap_or(());
			error!("IPMI control failed, trying to restore automatic fan control and exiting");
			::std::process::exit(1);
		}
	}
}

fn set_fan_manual(manual: bool, shadow: bool) -> Result<()> {
	if shadow {
		trace!("Shadow: Setting manual fan control to {}", manual);
		Ok(())
	} else {
		ipmi_set_fan_manual(manual)
	}
}

fn set_fan_speed(speed: f32, shadow: bool) -> Result<()> {
	if shadow {
		trace!("Shadow: Setting fan speed to {}", speed);
		Ok(())
	} else {
		ipmi_set_fan_speed(speed)
	}
}
