use std::io::{Error, ErrorKind, Result};

use crate::pid::*;
use crate::ipmi::*;

pub struct ControlLoop {
	pids: Vec<(PID, f32)>,
	pvs: Vec<IPMIRequest>
}

impl ControlLoop {
	pub fn new() -> ControlLoop {
		ControlLoop {
			pids: vec!(),
			pvs: vec!()
		}
	}

	pub fn add_control(&mut self, name: String, setpoint: f32, tuning: (f32,f32,f32), failsafe: f32) {
		self.pids.push((PID::new(setpoint, tuning), failsafe));
		self.pvs.push(IPMIRequest { name, status: IPMIValue::Unknown });
	}

	pub fn step(&mut self, elapsed: f32) -> Result<f32> {
		trace!("Step {}", elapsed);

		get_ipmi_values(&mut self.pvs)?;

		let mut max = 0.0;

		for ((pid, failsafe), pv) in self.pids.iter_mut().zip(self.pvs.iter()) {
			let output = match pv.status {
				IPMIValue::Invalid => Err(Error::new(ErrorKind::InvalidData, format!("{} is invalid", pv.name))),
				IPMIValue::Unknown => Err(Error::new(ErrorKind::InvalidData, format!("{} is not set", pv.name))),
				IPMIValue::Temp(temp) => if temp as f32 >= *failsafe {
						Err(Error::new(ErrorKind::InvalidData, format!("failsafe of {} exceeded: {}", failsafe, temp)))
					} else {
						let temp = temp as f32;
						Ok(pid.update(temp, elapsed))
					},
				IPMIValue::RPM(_rpm) => Err(Error::new(ErrorKind::InvalidData, format!("cannot watch RPM value for {}", pv.name)))
			}?;

			trace!("Output for {} is {}", pv.name, output);

			max = output.max(max);
		}

		trace!("Max is {}", max);

		Ok(max)
	}
}
