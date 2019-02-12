use crate::metrics;

pub struct PID {
	setpoint: f32,
	prev_error: f32,
	i_acc: f32,
	k_factor: f32,
	i_factor: f32,
	d_factor: f32
}

impl PID {
	pub fn new(setpoint: f32, (k_factor,i_factor,d_factor): (f32, f32, f32)) -> PID {
		PID {
			setpoint,
			prev_error: 0.0,
			i_acc: 0.0,
			k_factor,
			i_factor,
			d_factor
		}
	}

	pub fn update(&mut self, current: f32, elapsed: f32, metric: Option<(String,&(String,String,Option<String>,Option<String>))>) -> f32 {
		let error = current - self.setpoint;

		self.i_acc += error * elapsed;
		self.i_acc = self.i_acc.max(0.0);

		let d_acc = if elapsed >= 0.001 {
				(error - self.prev_error) / elapsed //@todo: smoothing
			} else {
				0.0
			};
		self.prev_error = error;

		let p = error * self.k_factor;
		let i = self.i_acc * self.i_factor;
		let d = d_acc * self.d_factor;

		trace!("PID update ({},{},{}) ({},{},{}) = ({},{},{})", error, self.i_acc, d_acc, self.k_factor, self.i_factor, self.d_factor, p, i ,d);

		if let Some((ref metric, ref metric_config)) = metric {
			metrics::report_metric(&[("p".to_string(),p),("i".to_string(),i),("d".to_string(),d),("v".to_string(),p+i+d)], &[("pid".to_string(), metric.clone())], metric_config);
		}

		p + i + d
	}
}
