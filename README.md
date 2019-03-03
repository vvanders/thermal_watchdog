# Thermal Watchdog
This project uses IPMI for a PID driven fan control of Dell based servers to keep them from sounding like a jet engine in your homelab.

# WARNING - This project is proveded AS-IS WITH NO WARRANTY
Improperly configured this tool has the ability the **_PERMANENTLY DAMAGE_** your server! We are not responsible for any bugs, misbehavior or configurations that cause damage. It is your responsibility to propertly vet all code, understand the pid control algorithm and configuration parameters of this software.

# Installation
1. If you don't have it, install the Rust compiler: ```curl https://sh.rustup.rs -sSf | sh```
2. ```git clone https://github.com/vvanders/thermal_watchdog.git```
3. ```cd thermal_watchdog```
4. ```cargo build --release```
5. ```(sudo) cargo run --release -- install```
6. ```(sudo) apt install ipmitool```
7. ```(sudo) systemctl start thermal_watchdog```
8. ```(sudo) systemctl enable thermal_watchdog``` - If you want thermal_watchdog to start on boot.

Note that Thermal Watchdog starts in *Shadow Mode* where it will run but all IPMI commands are not actually comitted. To enable fan control read through the following configuration section.

# Configuration
After Thermal Watchdog is installed it will read from a configuration file at "/etc/thermal_watchdog.conf" that follows TOML syntax.

```
[metrics]
influx_user="admin"
influx_pw="influx"
influx_addr="http://localhost:8086"
influx_db="twd"

[pid]
k_factor = 0.025
i_factor = 0.000001
d_factor = 0
min = 5

[[controls]]
name = "Exhaust Temp"
setpoint = 40.0
failsafe = 60.0

[[controls]]
name = "Temp"
setpoint = 55.0
failsafe = 70.0

[[controls]]
name = "Temp"
setpoint = 55.0
failsafe = 70.0
```

## pid section
The ```pid``` section controls the core PID(Proportonal, Integral, Derivative) algorithm used to keep a set of temperature sensors under a specific setpoint.

PID control works based on an ```error value``` derived from a sepcific ```setpoint```(target temperature) and ```process variable```(temperature sensor). The ```error value``` is simply ```error value = (process variable) - setpoint```. For each temperature sensor an individual PID controller is created from a set of common parameters.

* ```k_factor```: *Proportonal* based control. For each degree a specific sensor is over the ```setpoint``` this control will add ```value``` to the total fan input.
* ```i_factor```: *Integral* based control. Accumulates ```error value``` over time to find a steady-state value when temperature is constant.
* ```d_factor```: *Derivative* based control. Adds/Subtracts to the current fan control based on the rate of change in ```error value```. Helps prevent overshooting but generally can be left at 0 unless you have very spiky loads. By default tracks the difference in the last ```5``` sample points.
* ```filter_points```: Number of historical points to consider in ```d_factor```. Larger values here make ```d_factor``` smoother, lower values make ```d_factor``` respond quicker.
* ```min```: Sets a minimum fan speed(0-100) regardless of PID output.

### PID deviations from *classic* model

In order to prevent the algorithm misbehaving in destructive ways the following changes apply from a "classic" PID control:
* The accumulator for **I** Factor is clamped to ```-0.25``` to ```1.0```.
* Final output to control is the ```max(...)``` of all current PID controllers.
