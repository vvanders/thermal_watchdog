use hyper::rt;
use hyper::{Body, Client, Request};

use futures::future;

pub fn report_metric(event: &[(String,f32)], tags: &[(String,String)], (host,db,user,pw): (&str,&str,&str,&str)) {
	let fields = event.iter().map(|(n,v)| format!("{}={}", n.replace(" ", "\\ "), v))
					.fold("".to_string(), |acc,v| {
						if acc.len() > 0 {
							acc + "," + v.as_str()
						} else {
							v
						}
					});
	let tags = tags.iter().fold("".to_string(), |acc, (n,v)| {
		acc + "," + n.replace(" ", "\\ ").as_str() + "=" + v.replace(" ", "\\ ").as_str()
	});
	let formatted = format!("thermal_watchdog{} {}", tags, fields);

	trace!("Submitting metric: {}", formatted);

	let client = Client::builder()
		.keep_alive(false)
		.build_http();
	let req = Request::post(format!("{}/write?db={}&u={}&p={}", host, db, user, pw))
		.body(Body::from(formatted))
		.expect("Failed to build request");

	use rt::{Future, Stream};
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
		.map_err(|e| error!("Unable to submit metrics: {}", e));

	rt::run(fut);
}
