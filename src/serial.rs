use eframe::egui::Color32;
use preferences::Preferences;
use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use std::io::{BufRead, BufReader};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use defmt_decoder::{DecodeError, Frame, Locations, Table};


use crate::color_picker::COLORS;
//use std::fmt::Write; // Import the Write trait for String
use crate::data::{get_epoch_ms, SerialDirection};
use crate::{Packet, APP_INFO, PREFERENCES_KEY_SERIAL};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialDevices {
    pub devices: Vec<Device>,
    pub labels: Vec<Vec<String>>,
    pub highlight_labels: Vec<Vec<String>>,
    pub colors: Vec<Vec<Color32>>,
    pub color_vals: Vec<Vec<f32>>,
    pub number_of_plots: Vec<usize>,
    pub number_of_highlights: Vec<usize>,
}

impl Default for SerialDevices {
    fn default() -> Self {
        SerialDevices {
            devices: vec![Device::default()],
            labels: vec![vec!["Column 0".to_string()]],
            highlight_labels: vec![vec!["".to_string()]],
            colors: vec![vec![COLORS[0]]],
            color_vals: vec![vec![0.0]],
            number_of_plots: vec![1],
            number_of_highlights: vec![1],
        }
    }
}

pub fn load_serial_settings() -> SerialDevices {
    SerialDevices::load(&APP_INFO, PREFERENCES_KEY_SERIAL).unwrap_or_else(|_| {
        let serial_configs = SerialDevices::default();
        // save default settings
        save_serial_settings(&serial_configs);
        serial_configs
    })
}

pub fn save_serial_settings(serial_configs: &SerialDevices) {
    if serial_configs
        .save(&APP_INFO, PREFERENCES_KEY_SERIAL)
        .is_err()
    {
        log::error!("failed to save gui_settings");
    }
}

pub fn clear_serial_settings() {
    let serial_configs = SerialDevices::default();
    if serial_configs
        .save(&APP_INFO, PREFERENCES_KEY_SERIAL)
        .is_err()
    {
        log::error!("failed to clear gui_settings");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub name: String,
    pub baud_rate: u32,
    pub data_bits: DataBits,
    pub flow_control: FlowControl,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub timeout: Duration,
}

impl Default for Device {
    fn default() -> Self {
        Device {
            name: "".to_string(),
            baud_rate: 9600,
            data_bits: DataBits::Eight,
            flow_control: FlowControl::None,
            parity: Parity::None,
            stop_bits: StopBits::One,
            timeout: Duration::from_millis(0),
        }
    }
}

fn serial_write(
    port: &mut BufReader<Box<dyn SerialPort>>,
    cmd: &[u8],
) -> Result<usize, std::io::Error> {
    let write_port = port.get_mut();
    write_port.write(cmd)
}

fn serial_read(
    port: &mut BufReader<Box<dyn SerialPort>>,
    serial_buf: &mut String,
) -> Result<usize, std::io::Error> {
    port.read_line(serial_buf)
}
/*
pub fn serial_thread(
    send_rx: Receiver<String>,
    raw_data_tx: Sender<Packet>,
    device_lock: Arc<RwLock<Device>>,
    devices_lock: Arc<RwLock<Vec<String>>>,
    connected_lock: Arc<RwLock<bool>>,
) {
    let mut last_connected_device = Device::default();

    loop {
        let _not_awake = keepawake::Builder::default()
            .display(false)
            .reason("Serial Connection")
            .app_name("Serial Monitor")
            .create();

        if let Ok(mut connected) = connected_lock.write() {
            *connected = false;
        }

        let device = get_device(&devices_lock, &device_lock, &last_connected_device);

        let mut port = match serialport::new(&device.name, device.baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
        {
            Ok(p) => {
                if let Ok(mut connected) = connected_lock.write() {
                    *connected = true;
                }

                log::info!(
                    "Connected to serial port: {} @ baud = {}",
                    device.name,
                    device.baud_rate
                );

                BufReader::new(p)
            }
            Err(err) => {
                if let Ok(mut write_guard) = device_lock.write() {
                    write_guard.name.clear();
                }
                log::error!("Error connecting: {}", err);
                continue;
            }
        };

        let t_zero = Instant::now();

        // Load the defmt symbol table (you need to provide the ELF file path)
        let elf_path = "D:/Study/GP/ESP/async_main.elf"; // Replace with the actual path to your ELF file
        let elf_data = match std::fs::read(elf_path) {
            Ok(data) => data,
            Err(e) => {
                log::error!("Failed to read ELF file: {}", e);
                break;
            }
        };

        let table = match Table::parse(&elf_data) {
            Ok(t) => t,
            Err(e) => {
                log::error!("Failed to parse defmt table: {}", e);
                break;
            }
        };

        let _awake = keepawake::Builder::default()
            .display(true)
            .reason("Serial Connection")
            .app_name("Serial Monitor")
            .create();

        'connected_loop: loop {
            let devices = available_devices();
            if let Ok(mut write_guard) = devices_lock.write() {
                *write_guard = devices.clone();
            }

            if disconnected(&device, &devices, &device_lock, &mut last_connected_device) {
                break 'connected_loop;
            }

            perform_writes(&mut port, &send_rx, &raw_data_tx, t_zero);
            if let Some(ref table) = table {
                perform_reads(&mut port, &raw_data_tx, t_zero, table);
            } else {
                log::error!("Defmt table is not available");
            }

            //std::thread::sleep(Duration::from_millis(10));
        }
        std::mem::drop(port);
    }
}*/

pub fn serial_thread(
    send_rx: Receiver<String>,
    raw_data_tx: Sender<Packet>,
    device_lock: Arc<RwLock<Device>>,
    devices_lock: Arc<RwLock<Vec<String>>>,
    connected_lock: Arc<RwLock<bool>>,
) {
    let mut last_connected_device = Device::default();

    loop {
        let _not_awake = keepawake::Builder::default()
            .display(false)
            .reason("Serial Connection")
            .app_name("Serial Monitor")
            //.app_reverse_domain("io.github.myprog")
            .create();

        if let Ok(mut connected) = connected_lock.write() {
            *connected = false;
        }

        let device = get_device(&devices_lock, &device_lock, &last_connected_device);

        let mut port = match serialport::new(&device.name, device.baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
        {
            Ok(p) => {
                if let Ok(mut connected) = connected_lock.write() {
                    *connected = true;
                }

                log::info!(
                    "Connected to serial port: {} @ baud = {}",
                    device.name,
                    device.baud_rate
                );

                BufReader::new(p)
            }
            Err(err) => {
                if let Ok(mut write_guard) = device_lock.write() {
                    write_guard.name.clear();
                }
                log::error!("Error connecting: {}", err);
                continue;
            }
        };

        let t_zero = Instant::now();

        let _awake = keepawake::Builder::default()
            .display(true)
            .reason("Serial Connection")
            .app_name("Serial Monitor")
            //.app_reverse_domain("io.github.myprog")
            .create();

        'connected_loop: loop {
            let devices = available_devices();
            if let Ok(mut write_guard) = devices_lock.write() {
                *write_guard = devices.clone();
            }

            if disconnected(&device, &devices, &device_lock, &mut last_connected_device) {
                break 'connected_loop;
            }

            perform_writes(&mut port, &send_rx, &raw_data_tx, t_zero);
            perform_reads(&mut port, &raw_data_tx, t_zero);

            //std::thread::sleep(Duration::from_millis(10));
        }
        std::mem::drop(port);
    }
}


fn available_devices() -> Vec<String> {
    serialport::available_ports()
        .unwrap()
        .iter()
        .map(|p| p.port_name.clone())
        .collect()
}

fn get_device(
    devices_lock: &Arc<RwLock<Vec<String>>>,
    device_lock: &Arc<RwLock<Device>>,
    last_connected_device: &Device,
) -> Device {
    loop {
        let devices = available_devices();
        if let Ok(mut write_guard) = devices_lock.write() {
            *write_guard = devices.clone();
        }

        // do reconnect
        if devices.contains(&last_connected_device.name) {
            if let Ok(mut device) = device_lock.write() {
                device.name = last_connected_device.name.clone();
                device.baud_rate = last_connected_device.baud_rate;
            }
            return last_connected_device.clone();
        }

        if let Ok(device) = device_lock.read() {
            if devices.contains(&device.name) {
                return device.clone();
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

//disconnection logic
fn disconnected(
    device: &Device,
    devices: &[String],
    device_lock: &Arc<RwLock<Device>>,
    last_connected_device: &mut Device,
) -> bool {
    // disconnection by button press
    if let Ok(read_guard) = device_lock.read() {
        if device.name != read_guard.name {
            *last_connected_device = Device::default();
            log::info!("Disconnected from serial port: {}", device.name);
            return true;
        }
    }

    // other types of disconnection (e.g. unplugging, power down)
    if !devices.contains(&device.name) {
        if let Ok(mut write_guard) = device_lock.write() {
            write_guard.name.clear();
        }
        *last_connected_device = device.clone();
        log::error!("Device has disconnected from serial port: {}", device.name);
        return true;
    };
    false
}

fn perform_writes(
    port: &mut BufReader<Box<dyn SerialPort>>,
    send_rx: &Receiver<String>,
    raw_data_tx: &Sender<Packet>,
    t_zero: Instant,
) {
    
    if let Ok(cmd) = send_rx.try_recv() {
        //fares behbed
        //println!("Sending command: {}", cmd); // <--- this is what you want
        if cmd == "__RESET__\r\n" {
            let inner = port.get_mut();
            println!("Sending command: {}", cmd); // <--- this is what you want
            // This triggers EN=LOW (reset), IO0=LOW (bootloader)
            inner.write_data_terminal_ready(false); // EN = HIGH
            std::thread::sleep(std::time::Duration::from_millis(100));

            inner.write_request_to_send(true);      // IO0 = LOW
            inner.write_data_terminal_ready(false); // EN = HIGH again (redundant)
            inner.write_request_to_send(true);      // IO0 = LOW again

            std::thread::sleep(std::time::Duration::from_millis(100));

            inner.write_request_to_send(false);
            return; // don't send "_RESET_" to the device!
        }
        //my own habed
        // Handle Ctrl+C
        if cmd == "__CTRLC__\r\n" {
            let inner = port.get_mut();
            println!("Sending Ctrl+C (0x03)");
            if let Err(e) = inner.write_all(&[0x03]) {
                log::error!("Error sending Ctrl+C: {e}");
                return;
            }

            let packet = Packet {
                relative_time: Instant::now().duration_since(t_zero).as_millis() as f64,
                absolute_time: get_epoch_ms() as f64,
                direction: SerialDirection::Send,
                payload: String::from("Ctrl+R (0x03)"),
            };
            raw_data_tx
                .send(packet)
                .expect("failed to send raw data (Ctrl+C)");
            return;
        }
        //end of fares habed
        if let Err(e) = serial_write(port, cmd.as_bytes()) {
            log::error!("Error sending command: {e}");
            return;
        }
        
        let packet = Packet {
            relative_time: Instant::now().duration_since(t_zero).as_millis() as f64,
            absolute_time: get_epoch_ms() as f64,
            direction: SerialDirection::Send,
            payload: cmd,
        };
        raw_data_tx
            .send(packet)
            .expect("failed to send raw data (cmd)");
    }
}

fn perform_reads(
    port: &mut BufReader<Box<dyn SerialPort>>,
    raw_data_tx: &Sender<Packet>,
    t_zero: Instant,
) {
    let mut buf = "".to_string();
    match serial_read(port, &mut buf) {
        Ok(_) => {
            let delimiter = if buf.contains("\r\n") { "\r\n" } else { "\0\0" };
            buf.split_terminator(delimiter).for_each(|s| {
                let packet = Packet {
                    relative_time: Instant::now().duration_since(t_zero).as_millis() as f64,
                    absolute_time: get_epoch_ms() as f64,
                    direction: SerialDirection::Receive,
                    payload: s.to_owned(),
                };
                raw_data_tx.send(packet).expect("failed to send raw data");
            });
        }
        // Timeout is ok, just means there is no data to read
        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
        Err(e) => {
            log::error!("Error reading: {:?}", e);
        }
    }
}

/*
fn perform_reads(
    port: &mut BufReader<Box<dyn SerialPort>>,
    raw_data_tx: &Sender<Packet>,
    t_zero: Instant,
    table: &Table, // Add the defmt `Table` as a parameter
) {
    let mut buf = String::new();
    match serial_read(port, &mut buf) {
        Ok(_) => {
            let bytes = buf.trim().as_bytes();
            match table.decode(bytes) {
                Ok((frame, _)) => {
                    // Decode the defmt frame into a human-readable message
                    let mut decoded_message = String::new();
                    if let Err(e) = write!(&mut decoded_message, "{}", frame.display(true)) {
                        log::error!("Failed to format decoded message: {:?}", e);
                        return;
                    }

                    // Send the decoded message as a Packet
                    let packet = Packet {
                        relative_time: Instant::now().duration_since(t_zero).as_millis() as f64,
                        absolute_time: get_epoch_ms() as f64,
                        direction: SerialDirection::Receive,
                        payload: decoded_message,
                    };
                    raw_data_tx.send(packet).expect("failed to send raw data");
                }
                Err(DecodeError::UnexpectedEof) => {
                    log::warn!("Incomplete defmt frame received");
                }
                Err(e) => {
                    log::error!("Failed to decode defmt frame: {:?}", e);
                }
            }
        }
        // Timeout is ok, just means there is no data to read
        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
        Err(e) => {
            log::error!("Error reading: {:?}", e);
        }
    }
}*/

