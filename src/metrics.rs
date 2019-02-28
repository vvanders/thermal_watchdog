use std::sync::mpsc;
use std::thread;

use hyper::rt;
use hyper::{Body, Client, Request};

use futures::future;

pub enum MetricEvent {
	Metric(String),
	Exit
}

pub type MetricSender = mpsc::Sender<MetricEvent>;

pub fn init_metric_thread(config: Option<(String,String,Option<String>,Option<String>)>) -> mpsc::Sender<MetricEvent> {
	let (send,recv) = mpsc::channel();

	thread::spawn(move || {
		loop {
			match recv.recv() {
				Ok(MetricEvent::Metric(mut event)) => {
					loop {
						match recv.try_recv() {
							Ok(MetricEvent::Metric(next_event)) => {
								event += "\n";
								event += next_event.as_str();
							},
							Ok(MetricEvent::Exit) | Err(mpsc::TryRecvError::Disconnected) => {
								info!("Shutting down metrics thread");
								return
							},
							Err(mpsc::TryRecvError::Empty) => break
						}
					}

					if let Some(config) = config.as_ref() {
						send_metric(config, event);
					}
				},
				Ok(MetricEvent::Exit) | Err(_) => {
					info!("Shutting down metrics thread");
					return
				}
			};
		}
	});

	send
}

fn send_metric((host,db,user,pw): &(String,String,Option<String>,Option<String>), event: String) {
	let client = Client::builder()
		.keep_alive(false)
		.build_http();

	let user = user.as_ref().map(|v| format!("&u={}",v)).unwrap_or(String::new());
	let pw = pw.as_ref().map(|v| format!("&p={}",v)).unwrap_or(String::new());
	let req = Request::post(format!("{}/write?db={}{}{}", host, db, user, pw))
		.body(Body::from(event))
		.expect("Failed to build request");

	use rt::{Future, Stream};
	use tokio::prelude::FutureExt;

	let fut = client.request(req)
		.and_then(|r| {
			if r.status().is_success() {
				trace!("Successful metrics submission");
			} else {
				error!("Failed to submit metrics, server returned {} code", r.status().as_u16());
			}

			r.into_body().for_each(|chunk| {
				trace!("Metric submit body: {:?}", chunk);
				future::ok(())
			})
		})
		.timeout(::std::time::Duration::from_secs(2))
		.map_err(|e| error!("Unable to submit metrics: {}", e));

	rt::run(fut);
}

pub fn report_metric(event: &[(String,f32)], tags: &[(String,String)], sender: &mpsc::Sender<MetricEvent>) {
	let hostname = get_hostname()
		.map(|v| format!(",hostname={}", v))
		.unwrap_or_else(|e| {
			error!("Unable to include hostname: {}", e);
			String::new()
		});

	let fields = event.iter().map(|(n,v)| format!("{}={}", n.replace(" ", "\\ "), v))
					.fold(String::new(), |acc,v| {
						if acc.len() > 0 {
							acc + "," + v.as_str()
						} else {
							v
						}
					});
	let tags = tags.iter().fold(hostname, |acc, (n,v)| {
		acc + "," + n.replace(" ", "\\ ").as_str() + "=" + v.replace(" ", "\\ ").as_str()
	});
	let formatted = format!("thermal_watchdog{} {}", tags, fields);

	trace!("Submitting metric: {}", formatted);

	sender.send(MetricEvent::Metric(formatted))
		.unwrap_or_else(|e| {
			error!("Unable to write metric to sender: {:?}", e);
		});
}

fn get_hostname() -> Result<String,String> {
	let cmd = ::std::process::Command::new("hostname")
		.output()
		.map_err(|e| format!("Unable to run commandL {:?}", e))?;

	if !cmd.status.success() {
		return Err("Command returned non-zero exit code".to_string())
	}

	::std::str::from_utf8(&cmd.stdout[..])
		.map(|v| v.trim().to_string())
		.map_err(|e| format!("Unable to get command output: {:?}", e))
}

pub fn get_proc_usage() -> Result<f32,String> {
	let stat1 = get_cpu_stats()?;
	::std::thread::sleep(::std::time::Duration::from_millis(100));
	let stat2 = get_cpu_stats()?;

	let user = stat2.0 - stat1.0;
	let system = stat2.1 - stat1.1;
	let idle = stat2.2 - stat1.2;

	Ok((user + system) as f32 / (user + system + idle) as f32)
}

fn get_cpu_stats() -> Result<(usize,usize,usize),String> {
	let mut content = String::new();
	let mut file = ::std::fs::File::open("/proc/stat")
		.map_err(|e| format!("unable to open /proc/stat: {:?}", e))?;

	use ::std::io::Read;
	file.read_to_string(&mut content)
		.map_err(|e| format!("unable to read /proc/stat: {:?}", e))?;

	if let Some(line) = content.lines().next() {
		let split = line.split_whitespace().collect::<Vec<_>>();

		if split.len() < 5 || split[0] != "cpu" {
			return Ok((0,0,0))
		}

		let stats = split.iter()
			.skip(1)
			.map(|v| usize::from_str_radix(v, 10)
					.map_err(|_| format!("Unable to parse \"{}\"", v)))
			.collect::<Result<Vec<_>,_>>()?;

		Ok((stats[0], stats[2], stats[3]))
	} else {
		return Ok((0,0,0))
	}
}
