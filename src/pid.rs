use crate::metrics;

pub struct PID {
	setpoint: f32,
	i_acc: f32,
	k_factor: f32,
	i_factor: f32,
	d_factor: f32,
	filter_points: usize,
	d_filter: Vec<(f32,f32)>
}

impl PID {
	pub fn new(setpoint: f32, (k_factor,i_factor,d_factor): (f32, f32, f32), filter_points: usize) -> PID {
		PID {
			setpoint,
			i_acc: 0.0,
			k_factor,
			i_factor,
			d_factor,
			filter_points,
			d_filter: vec!()
		}
	}

	pub fn update(&mut self, current: f32, elapsed: f32, (metric, metric_sender): (String, &metrics::MetricSender)) -> f32 {
		let error = current - self.setpoint;

		self.i_acc += error * elapsed;
		self.i_acc = self.i_acc.max(-0.25);

		self.d_filter.push((elapsed, error));
		let d_acc = calc_diff(&mut self.d_filter, self.filter_points);

		let p = error * self.k_factor;
		let i = self.i_acc * self.i_factor;
		let d = d_acc * self.d_factor;

		trace!("PID update ({},{},{}) ({},{},{}) = ({},{},{})", error, self.i_acc, d_acc, self.k_factor, self.i_factor, self.d_factor, p, i ,d);

		metrics::report_metric(&[("p".to_string(),p),("i".to_string(),i),("d".to_string(),d),("v".to_string(),p+i+d)], &[("pid".to_string(), metric.clone())], metric_sender);

		p + i + d
	}
}

fn calc_diff(data: &mut Vec<(f32, f32)>, filter_points: usize) -> f32 {
	while data.len() > filter_points + 1 {
		data.remove(0);
	}

	data.iter().zip(data.iter().skip(1))
		.map(|((_,v1),(t,v2))| {
			(v2 - v1) / t
		})
		.sum()
}
