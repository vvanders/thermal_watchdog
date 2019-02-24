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
