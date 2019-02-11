#[macro_use]
extern crate log;

mod pid;
mod ipmi;
mod control;
mod metrics;

use ipmi::*;
use control::*;

use env_logger;
use clap::{Arg, App, SubCommand};
use ctrlc;

use std::time::Instant;
use std::io::Result;

fn main() {
	env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace"))
		.filter(Some("tokio_reactor"), log::LevelFilter::Info)
		.filter(Some("tokio_threadpool"), log::LevelFilter::Info)
		.filter(Some("mio"), log::LevelFilter::Info)
		.filter(Some("want"), log::LevelFilter::Info)
		.filter(Some("hyper"), log::LevelFilter::Info)
		.init();

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
						.long("influx_pw")
						.takes_value(true)
						.help("InfluxDB password"))
					.arg(Arg::with_name("influx_db")
						.short("d")
						.long("influx_db")
						.takes_value(true)
						.help("InfluxDB database"))
					.subcommand(SubCommand::with_name("install")
						.about("Installs Thermal Watchdog as systemd service"))
		.get_matches();

	if let Some(_) = matches.subcommand_matches("install") {
		install();
		return
	}

	let shadow = !matches.is_present("live");

	ctrlc::set_handler(move || {
		info!("Signal received, aborting and resetting IPMI control");
		set_fan_manual(false, shadow, None).unwrap_or(());
		::std::process::exit(1);
	}).expect("Unable to set signal handler");

	let metrics = match (matches.value_of("influx_addr"), matches.value_of("influx_db"), matches.value_of("influx_user"), matches.value_of("influx_pw")) {
		(Some(addr),Some(db),Some(user),Some(pw)) => {
			trace!("Enabling metrics");
			Some((addr,db,user,pw))
		},
		_ => None
	};

	main_loop(shadow, metrics);
}

fn main_loop(shadow: bool, metrics: Option<(&str,&str,&str,&str)>) {
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

	use systemd::daemon;
	daemon::notify(false, [(daemon::STATE_READY,"1")].iter()).unwrap_or(false);
	
	let mut manual = false;
	let mut last_update = Instant::now();
	loop {
		let now = Instant::now();
		let duration = now.duration_since(last_update);
		last_update = now;

		let elapsed = (duration.as_secs() * 1000 + duration.subsec_millis() as u64) as f32;
		let loop_result = control_loop.step(elapsed, metrics);

		let set_result = match loop_result {
			Ok(control) => {
				let enable = if !manual {
					info!("Enabling manual fan control");
					set_fan_manual(true, shadow, metrics)
						.and_then(|_| {
							manual = true;
							Ok(())
						})
				} else {
					Ok(())
				};

				enable.and_then(|_| set_fan_speed(control, shadow, metrics))
			},
			Err(e) => {
				error!("Unable to run control, resetting to manual: {}", e);
				set_fan_manual(false, shadow, metrics).and_then(|_| {
					manual = false;
					Ok(())
				})
			}
		};

		if let Err(_) = set_result {
			error!("IPMI control failed, trying to restore automatic fan control and exiting");

			match set_fan_manual(false, shadow, metrics) {
				Ok(_) => info!("Restored automatic fan control"),
				Err(e) => error!("Failed to restore automatic fan control: {:?}", e)
			}

			daemon::notify(false, [(daemon::STATE_STOPPING,"1")].iter()).unwrap_or(false);

			::std::process::exit(1);
		}

		daemon::notify(false, [(daemon::STATE_WATCHDOG,"1")].iter()).unwrap_or(false);
	}
}

fn set_fan_manual(manual: bool, shadow: bool, metrics: Option<(&str,&str,&str,&str)>) -> Result<()> {
	if let Some(metric_config) = metrics {
		let value = if manual {
			1.0
		} else {
			0.0
		};

		metrics::report_metric(&[("manual control".to_string(), value)], &[], metric_config);
	}

	if shadow {
		trace!("Shadow: Setting manual fan control to {}", manual);
		Ok(())
	} else {
		ipmi_set_fan_manual(manual)
	}
}

fn set_fan_speed(speed: f32, shadow: bool, metrics: Option<(&str,&str,&str,&str)>) -> Result<()> {
	if let Some(metric_config) = metrics {
		metrics::report_metric(&[("fan speed".to_string(), speed)], &[], metric_config);
	}

	if shadow {
		trace!("Shadow: Setting fan speed to {}", speed);
		Ok(())
	} else {
		ipmi_set_fan_speed(speed)
	}
}

fn install() {
	let exe_path = "/usr/sbin/thermal_watchdog";
	let exe = ::std::env::current_exe().expect("Unable to determine binary location");

	::std::fs::copy(exe, exe_path).expect("Unable to copy thermal_watchdog to /usr/sbin, are you running as root?");

	let service_conf = 
r#"[Unit]
Description=Thermal Watchdog

[Service]
Type=notify
ExecStart=/usr/sbin/thermal_watchdog
ExecStopPost=/usr/bin/ipmitool raw 0x30 0x30 0x01 0x01
Restart=on-failure
WatchdogSec=10

[Install]
WantedBy=multi-user.target
"#;
	let conf_path = "/etc/systemd/system/thermal_watchdog.service";

	let mut service_file = ::std::fs::File::create(conf_path).expect(format!("Unable to open {}, are you running as root?", conf_path).as_str());

	use std::io::Write;
	service_file.write_all(service_conf.as_bytes()).expect("Unable to write service file, are you running as root?");

}
