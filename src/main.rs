#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

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
					.arg(Arg::with_name("config")
						.long("config")
						.short("c")
						.default_value("/etc/thermal_watchdog.toml")
						.help("Path to configuration TOML"))
					.arg(Arg::with_name("influx_addr")
						.short("a")
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

	let config_file = matches.value_of("config").expect("no config defined");

	let mut config = parse_config(config_file);

	let shadow = !matches.is_present("live");

	ctrlc::set_handler(move || {
		info!("Signal received, aborting and resetting IPMI control");
		set_fan_manual(false, shadow, None).unwrap_or(());
		::std::process::exit(1);
	}).expect("Unable to set signal handler");

	match (matches.value_of("influx_addr").map(|v| v.to_string()), matches.value_of("influx_db").map(|v| v.to_string())) {
		(Some(influx_addr),Some(influx_db)) => {
			trace!("Enabling metrics");

			let (influx_user,influx_pw) = if let Some(prev_metrics) = config.metrics {
				(matches.value_of("influx_user").map(|v| v.to_string()).or(prev_metrics.influx_user),
					 matches.value_of("influx_pw").map(|v| v.to_string()).or(prev_metrics.influx_pw))
			} else {
				(None, None)
			};

			config.metrics = Some(AppMetricConfig {
				influx_addr,
				influx_db,
				influx_user,
				influx_pw
			});
		},
		_ => ()
	}

	main_loop(shadow, config);
}

#[derive(Deserialize)]
struct AppConfig {
	metrics: Option<AppMetricConfig>,
	pid: Option<AppPIDConfig>,
	controls: Option<Vec<AppControlConfig>>
}

#[derive(Deserialize)]
struct AppMetricConfig {
	influx_addr: String,
	influx_db: String,
	influx_user: Option<String>,
	influx_pw: Option<String>,
}

#[derive(Deserialize)]
struct AppPIDConfig {
	k_factor: f32,
	i_factor: f32,
	d_factor: f32,
	filter_points: Option<usize>,
	min: Option<usize>
}

#[derive(Deserialize,Clone)]
struct AppControlConfig {
	name: String,
	setpoint: f32,
	failsafe: f32
}

fn parse_config(path: &str) -> AppConfig {
	info!("Loading config file at {}", path);

	let result = ::std::fs::File::open(path)
		.map_err(|e| format!("Unable to open config file: {:?}", e))
		.and_then(|v| {
				let mut content = String::new();
				let mut buf = ::std::io::BufReader::new(v);
				use ::std::io::Read;
				buf.read_to_string(&mut content)
					.map_err(|e| format!("Unable to read config file: {:?}", e))?;
				Ok(content)
			})
		.and_then(|v| toml::from_str(v.as_str())
			.map_err(|e| format!("Unable to parse toml: {:?}", e)));

	match result {
		Ok(v) => v,
		Err(e) => {
			info!("{}", e);
			AppConfig {
				metrics: None,
				pid: None,
				controls: None
			}
		}
	}
}

fn main_loop(shadow: bool, config: AppConfig) {
	let controls = if let Some(controls) = config.controls {
		controls.clone()
	} else {
		vec!(
			AppControlConfig {
				name: "Exhaust Temp".to_string(),
				setpoint: 40.0,
				failsafe: 60.0
			},
			AppControlConfig {
				name: "Temp".to_string(),
				setpoint: 55.0,
				failsafe: 65.0
			},
			AppControlConfig {
				name: "Temp".to_string(),
				setpoint: 55.0,
				failsafe: 65.0
			}
		)
	};

	let metrics_conf = if let Some(metrics) = config.metrics {
		Some((
			metrics.influx_addr.clone(),
			metrics.influx_db.clone(),
			metrics.influx_user.clone(),
			metrics.influx_pw.clone()
		))
	} else {
		None
	};

	let metrics = metrics::init_metric_thread(metrics_conf);
	let metrics = &metrics;

	if shadow {
		info!("TWD running in Shadow Mode, no IPMI commands will be issued");
	}

	let filter_points = config.pid.as_ref().and_then(|v| v.filter_points).unwrap_or(5);
	let pid_settings = config.pid.as_ref()
		.map(|v| (v.k_factor, v.i_factor, v.d_factor))
		.unwrap_or((0.05, 0.000001, 0.0));

	let min_speed = config.pid.map(|v| v.min.unwrap_or(0)).unwrap_or(0) as f32 / 100.0;

	let mut control_loop = ControlLoop::new();

	for control in controls {
		control_loop.add_control(control.name.clone(), control.setpoint, pid_settings, filter_points, control.failsafe);
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
					set_fan_manual(true, shadow, Some(metrics))
						.and_then(|_| {
							manual = true;
							Ok(())
						})
				} else {
					Ok(())
				};

				let control = control.max(min_speed);

				enable.and_then(|_| set_fan_speed(control, shadow, metrics))
			},
			Err(e) => {
				error!("Unable to run control, resetting to manual: {}", e);
				set_fan_manual(false, shadow, Some(metrics)).and_then(|_| {
					manual = false;
					Ok(())
				})
			}
		};

		if let Err(_) = set_result {
			error!("IPMI control failed, trying to restore automatic fan control and exiting");

			match set_fan_manual(false, shadow, Some(metrics)) {
				Ok(_) => info!("Restored automatic fan control"),
				Err(e) => error!("Failed to restore automatic fan control: {:?}", e)
			}

			daemon::notify(false, [(daemon::STATE_STOPPING,"1")].iter()).unwrap_or(false);

			metrics.send(metrics::MetricEvent::Exit)
				.unwrap_or(());

			::std::process::exit(1);
		}

		match metrics::get_proc_usage() {
			Ok(cpu) => metrics::report_metric(&[("cpu_usage".to_string(), cpu)], &[], metrics),
			Err(e) => error!("Unable to report cpu usage: {}", e)
		}

		daemon::notify(false, [(daemon::STATE_WATCHDOG,"1")].iter()).unwrap_or(false);
	}
}

fn set_fan_manual(manual: bool, shadow: bool, metric_sender: Option<&metrics::MetricSender>) -> Result<()> {
	let value = if manual {
		1.0
	} else {
		0.0
	};

	if let Some(metric_sender) = metric_sender {
		metrics::report_metric(&[("manual control".to_string(), value)], &[], metric_sender);
	}

	if shadow {
		trace!("Shadow: Setting manual fan control to {}", manual);
		Ok(())
	} else {
		ipmi_set_fan_manual(manual)
	}
}

fn set_fan_speed(speed: f32, shadow: bool, metric_sender: &metrics::MetricSender) -> Result<()> {
	metrics::report_metric(&[("fan speed".to_string(), speed)], &[], metric_sender);

	if shadow {
		trace!("Shadow: Setting fan speed to {}", speed);
		Ok(())
	} else {
		ipmi_set_fan_speed(speed)
	}
}

fn install() {
	use std::path::Path;
	use std::io::Write;

	if !Path::new("/usr/bin/ipmitool").exists() {
		error!("Unable to find /usr/bin/ipmitool, please install with \"apt install ipmitool\"");
		return
	}

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

	if !Path::new(conf_path).exists() {
		let mut service_file = ::std::fs::File::create(conf_path).expect(format!("Unable to open {}, are you running as root?", conf_path).as_str());
		service_file.write_all(service_conf.as_bytes()).expect("Unable to write service file, are you running as root?");
	} else {
		info!("Skipped installing {} since it already exists", conf_path);
	}

	let toml_conf =
r#"#[metrics]
#influx_user="admin"
#influx_pw="influx"
#influx_addr="http://localhost:8086"
#influx_db="twd"

[pid]
k_factor = 0.025
i_factor = 0.000001
d_factor = 0.0
min = 5

[[controls]]
name = "Exhaust Temp"
setpoint = 40.0
failsafe = 60.0

[[controls]]
name = "Temp"
setpoint = 55.0
failsafe = 65.0

[[controls]]
name = "Temp"
setpoint = 55.0
failsafe = 65.0
"#;
	let toml_path = "/etc/thermal_watchdog.toml";

	if !Path::new(&toml_path).exists() {
		let mut toml_file = ::std::fs::File::create(toml_path).expect(format!("Unable to open {}, are you running as root?", toml_path).as_str());
		toml_file.write_all(toml_conf.as_bytes()).expect("Unable to write config file, are you running as root?");
	} else {
		info!("Skipped installing {} since it already exists", toml_path);
	}

}
