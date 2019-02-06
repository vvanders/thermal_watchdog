use std::process::Command;
use std::io::{Error, ErrorKind, Result};

#[derive(Debug)]
pub enum IPMIValue {
	Unknown,
	Invalid,
	Temp(i32),
	RPM(u32)
}

pub struct IPMIRequest {
	pub name: String,
	pub status: IPMIValue
}

pub fn get_ipmi_values(values: &mut Vec<IPMIRequest>) -> Result<()> {
	for value in values.iter_mut() {
		value.status = IPMIValue::Unknown;
	}

	let cmd = Command::new("ipmitool")
		.args(&["sdr", "list", "full"])
		.output()?;

	let output = ::std::str::from_utf8(&cmd.stdout[..])
		.map_err(|_| Error::new(ErrorKind::InvalidData, "Unable to parse command output, invalid utf-8"))?;

	let lines = output.lines();

	for line in lines {
		let mut cols = line.split("|");

		let name = cols.next();
		let value = cols.next();

		let name = name.map_or(Err(()), |v| Ok(v.trim()));
		let status = value.map_or(Err(()), |v| Ok(v.trim()));

		if let Ok((name, status)) = name.and_then(|n| status.map(|v| (n,parse_ipmi_value(n,v)))) {
			for value in values.iter_mut() {
				if let IPMIValue::Unknown = value.status {
					if value.name == name {
						trace!("Read {:?} for {}", status, name);
						value.status = status;
						break;
					}
				}
			}
		}
	}

	if !cmd.status.success() {
		return Err(Error::new(ErrorKind::InvalidData, "Command returned non-zero error code"));
	}

	Ok(())
}

fn parse_ipmi_value(name: &str, value: &str) -> IPMIValue {
	let first_ws = value.find(" ");

	if let Some(first_ws) = first_ws {
		let (data, label) = value.split_at(first_ws);

		let parsed = match label {
			" RPM" => u32::from_str_radix(data, 10)
						.map_err(|e| format!("Unable to parse {} for {}, {:?}", data, name, e))
						.map(|v| IPMIValue::RPM(v)),
			" degrees C" => i32::from_str_radix(data, 10)
						.map_err(|e| format!("Unable to parse {} for {}, {:?}", data, name, e))
						.map(|v| IPMIValue::Temp(v)),
			_ => Ok(IPMIValue::Unknown)
		};

		match parsed {
			Ok(v) => v,
			Err(e) => {
				warn!("Unable to parse ipmi entry: {}", e);
				IPMIValue::Invalid
			}
		}
	} else {
  		IPMIValue::Unknown
	}
}

pub fn ipmi_set_fan_manual(manual: bool) -> Result<()> {
	info!("Setting fan manual control: {}", manual);

	let enabled = if manual {
		"0x00"
	} else {
		"0x01"
	};

	let cmd = Command::new("ipmitool")
		.args(&["raw", "0x30", "0x30", "0x01", enabled])
		.output()?;

	if !cmd.status.success() {
		let output = ::std::str::from_utf8(&cmd.stdout[..]).unwrap_or("Invalid UTF-8");
		return Err(Error::new(ErrorKind::InvalidData, format!("Manual fan control failed, returned {}, {}", cmd.status.code().unwrap_or(-1), output)))
	}

	Ok(())
}

pub fn ipmi_set_fan_speed(speed: f32) -> Result<()> {
	info!("Setting fan speed to {}", speed);

	let scale = (speed * 100.0).ceil().min(100.0).max(0.0) as usize;
	let hex = format!("0x{:02x}", scale);

	info!("Setting fan speed to {} {}", hex, scale);

	let cmd = Command::new("ipmitool")
		.args(&["raw", "0x30", "0x30", "0x02", "0xff", hex.as_str()])
		.output()?;

	if !cmd.status.success() {
		let output = ::std::str::from_utf8(&cmd.stdout[..]).unwrap_or("Invalid UTF-8");
		return Err(Error::new(ErrorKind::InvalidData, format!("Fan control speed failed, returned {}, {}", cmd.status.code().unwrap_or(-1), output)))
	}

	Ok(())
}
