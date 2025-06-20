#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// hide console window on Windows in release
extern crate core;
extern crate csv;
extern crate preferences;
extern crate serde;

use std::cmp::max;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, RwLock};
use std::time::Duration;
use std::{env, thread};

use crate::data::{DataContainer, Packet};
use crate::gui::{load_gui_settings, MyApp, RIGHT_PANEL_WIDTH};
use crate::io::{open_from_csv, save_to_csv, FileOptions};
use crate::serial::{load_serial_settings, serial_thread, Device};
use eframe::egui::{vec2, ViewportBuilder, Visuals};
use eframe::{egui, icon_data};
use preferences::AppInfo;

mod color_picker;
mod custom_highlighter;
mod data;
mod gui;
mod io;
mod serial;
mod settings_window;
mod toggle;
mod update;

const APP_INFO: AppInfo = AppInfo {
    name: "Serial Monitor",
    author: "Linus Leo Stöckli",
};
const PREFERENCES_KEY: &str = "config/gui";
const PREFERENCES_KEY_SERIAL: &str = "config/serial_devices";

fn split(payload: &str) -> Vec<f32> {
    let mut split_data: Vec<&str> = vec![];
    for s in payload.split(':') {
        split_data.extend(s.split(','));
    }
    split_data
        .iter()
        .map(|x| x.trim())
        .flat_map(|x| x.parse::<f32>())
        .collect()
}

// fn extract_csi_data(payload: &str) -> Vec<i32> {
//     // Find the start of the "csi raw data" section
//     if let Some(start) = payload.find("csi raw data:") {
//         // Extract the part after "csi raw data:"
//         let raw_data = &payload[start + "csi raw data:".len()..];
        
//         // Remove brackets and split the numbers by commas
//         raw_data
//             .trim()
//             .trim_start_matches('[')
//             .trim_end_matches(']')
//             .split(',')
//             .filter_map(|x| x.trim().parse::<i32>().ok()) // Parse each number as i32
//             .collect()
//     } else {
//         // If "csi raw data" is not found, return an empty vector
//         vec![]
//     }
// }

// // Function to extract the "csi raw data" section from the payload
// // This function assumes that the payload is a string and that the "csi raw data" section is formatted correctly
// fn extract_csi_data(payload: &str) -> &str {
//     // Find the start of the "csi raw data" section
//     if let Some(start) = payload.find("csi raw data:") {
//         // Extract the part after "csi raw data:"
//         let raw_data = &payload[start + "csi raw data:".len()..];
        
//         // Trim and return the raw data section
//         raw_data.trim()
//     } else {
//         // If "csi raw data" is not found, return an empty string
//         ""
//     }
// }

fn main_thread(
    sync_tx: Sender<bool>,
    data_lock: Arc<RwLock<DataContainer>>,
    raw_data_rx: Receiver<Packet>,
    save_rx: Receiver<FileOptions>,
    load_rx: Receiver<PathBuf>,
    load_names_tx: Sender<Vec<String>>,
    clear_rx: Receiver<bool>,
) {
    // reads data from mutex, samples and saves if needed
    let mut data = DataContainer::default();
    let mut failed_format_counter = 0;

    let mut file_opened = false;

    loop {
        if let Ok(cl) = clear_rx.recv_timeout(Duration::from_millis(1)) {
            if cl {
                data = DataContainer::default();
                failed_format_counter = 0;
            }
        }
        if !file_opened {
            if let Ok(packet) = raw_data_rx.recv_timeout(Duration::from_millis(1)) {
                data.loaded_from_file = false;
                if !packet.payload.is_empty() {
                    sync_tx.send(true).expect("unable to send sync tx");
                    data.raw_traffic.push(packet.clone());
                    // let extracted_data = extract_csi_data(&packet.payload);
                    // let split_data = split(extracted_data);
                    let split_data = split(&packet.payload);
                    //log::debug!("split data: {:?}", split_data);
                    //println!("split data: {:?}", split_data);
                    
                    //here we might use defmt-print!!!!!!!!!!!!!!!
                    if data.dataset.is_empty() || failed_format_counter > 10 {
                        // resetting dataset
                        data.dataset = vec![vec![]; max(split_data.len(), 1)];
                        failed_format_counter = 0;
                        // log::error!("resetting dataset. split length = {}, length data.dataset = {}", split_data.len(), data.dataset.len());
                    } else if split_data.len() == data.dataset.len() {
                        // appending data
                        for (i, set) in data.dataset.iter_mut().enumerate() {
                            set.push(split_data[i]);
                            failed_format_counter = 0;
                        }
                        data.time.push(packet.relative_time);
                        data.absolute_time.push(packet.absolute_time);
                        if data.time.len() != data.dataset[0].len() {
                            // resetting dataset
                            data.time = vec![];
                            data.dataset = vec![vec![]; max(split_data.len(), 1)];
                        }
                    } else {
                        // not same length
                        failed_format_counter += 1;
                        // log::error!("not same length in main! length split_data = {}, length data.dataset = {}", split_data.len(), data.dataset.len())
                    }
                }
                // if !packet.payload.is_empty() {
                //     sync_tx.send(true).expect("unable to send sync tx");
                //     data.raw_traffic.push(packet.clone());
                
                //     // Extract CSI data directly
                //     let extracted_data = extract_csi_data(&packet.payload);
                
                //     // Use the extracted data directly instead of calling split
                //     if data.dataset.is_empty() || failed_format_counter > 10 {
                //         // Resetting dataset
                //         data.dataset = vec![vec![]; max(extracted_data.len(), 1)];
                //         failed_format_counter = 0;
                //     } else if extracted_data.len() == data.dataset.len() {
                //         // Appending data
                //         for (i, set) in data.dataset.iter_mut().enumerate() {
                //             set.push(extracted_data[i] as f32); // Convert i32 to f32 if needed
                //             failed_format_counter = 0;
                //         }
                //         data.time.push(packet.relative_time);
                //         data.absolute_time.push(packet.absolute_time);
                //         if data.time.len() != data.dataset[0].len() {
                //             // Resetting dataset
                //             data.time = vec![];
                //             data.dataset = vec![vec![]; max(extracted_data.len(), 1)];
                //         }
                //     } else {
                //         // Not same length
                //         failed_format_counter += 1;
                //     }
                // }
            }
        }
        if let Ok(fp) = load_rx.recv_timeout(Duration::from_millis(10)) {
            if let Some(file_ending) = fp.extension() {
                match file_ending.to_str().unwrap() {
                    "csv" => {
                        file_opened = true;
                        let mut file_options = FileOptions {
                            file_path: fp.clone(),
                            save_absolute_time: false,
                            save_raw_traffic: false,
                            names: vec![],
                        };
                        match open_from_csv(&mut data, &mut file_options) {
                            Ok(_) => {
                                log::info!("opened {:?}", fp);
                                load_names_tx
                                    .send(file_options.names)
                                    .expect("unable to send names on channel after loading");
                            }
                            Err(err) => {
                                file_opened = false;
                                log::error!("failed opening {:?}: {:?}", fp, err);
                            }
                        };
                    }
                    _ => {
                        file_opened = false;
                        log::error!("file not supported: {:?} \n Close the file to connect to a spectrometer or open another file.", fp);
                        continue;
                    }
                }
            } else {
                file_opened = false;
            }
        } else {
            file_opened = false;
        }

        if let Ok(mut write_guard) = data_lock.write() {
            *write_guard = data.clone();
        }

        if let Ok(csv_options) = save_rx.recv_timeout(Duration::from_millis(1)) {
            match save_to_csv(&data, &csv_options) {
                Ok(_) => {
                    log::info!("saved data file to {:?} ", csv_options.file_path);
                }
                Err(e) => {
                    log::error!(
                        "failed to save file to {:?}: {:?}",
                        csv_options.file_path,
                        e
                    );
                }
            }
        }
    }
}

fn main() {
    egui_logger::builder().init().unwrap();

    let gui_settings = load_gui_settings();
    let saved_serial_device_configs = load_serial_settings();

    let device_lock = Arc::new(RwLock::new(Device::default()));
    let devices_lock = Arc::new(RwLock::new(vec![gui_settings.device.clone()]));
    let data_lock = Arc::new(RwLock::new(DataContainer::default()));
    let connected_lock = Arc::new(RwLock::new(false));

    let (save_tx, save_rx): (Sender<FileOptions>, Receiver<FileOptions>) = mpsc::channel();
    let (load_tx, load_rx): (Sender<PathBuf>, Receiver<PathBuf>) = mpsc::channel();
    let (loaded_names_tx, loaded_names_rx): (Sender<Vec<String>>, Receiver<Vec<String>>) =
        mpsc::channel();
    let (send_tx, send_rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    let (clear_tx, clear_rx): (Sender<bool>, Receiver<bool>) = mpsc::channel();
    let (raw_data_tx, raw_data_rx): (Sender<Packet>, Receiver<Packet>) = mpsc::channel();
    let (sync_tx, sync_rx): (Sender<bool>, Receiver<bool>) = mpsc::channel();

    // // Simulated data stream
    // let simulated_data_tx = raw_data_tx.clone();
    // thread::spawn(move || {
    //     loop {
    //         let packet = Packet {
    //             payload: "12.34,56.78:90.12,34.56".to_string(),
    //             relative_time: 0.0,
    //             absolute_time: 0.0,
    //             direction: data::SerialDirection::Receive,
    //         };
    //         simulated_data_tx.send(packet).expect("Failed to send simulated packet");
    //         thread::sleep(Duration::from_secs(1));
    //     }
    // });
    
    let serial_device_lock = device_lock.clone();
    let serial_devices_lock = devices_lock.clone();
    let serial_connected_lock = connected_lock.clone();

    let _serial_thread_handler = thread::spawn(|| {
        serial_thread(
            send_rx,
            raw_data_tx,
            serial_device_lock,
            serial_devices_lock,
            serial_connected_lock,
        );
    });

    let main_data_lock = data_lock.clone();

    let _main_thread_handler = thread::spawn(|| {
        main_thread(
            sync_tx,
            main_data_lock,
            raw_data_rx,
            save_rx,
            load_rx,
            loaded_names_tx,
            clear_rx,
        );
    });

    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        load_tx
            .send(PathBuf::from(&args[1]))
            .expect("failed to send file");
    }

    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_drag_and_drop(true)
            .with_inner_size(vec2(gui_settings.x, gui_settings.y))
            .with_min_inner_size(vec2(2.0 * RIGHT_PANEL_WIDTH, 2.0 * RIGHT_PANEL_WIDTH))
            .with_icon(
                icon_data::from_png_bytes(&include_bytes!("../icons/icon.png")[..]).unwrap(),
            ),
        ..Default::default()
    };

    let gui_data_lock = data_lock;
    let gui_device_lock = device_lock;
    let gui_devices_lock = devices_lock;
    let gui_connected_lock = connected_lock;

    if let Err(e) = eframe::run_native(
        "Serial Monitor",
        options,
        Box::new(|ctx| {
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            ctx.egui_ctx.set_fonts(fonts);
            ctx.egui_ctx.set_visuals(Visuals::dark());
            egui_extras::install_image_loaders(&ctx.egui_ctx);

            let repaint_signal = ctx.egui_ctx.clone();
            thread::spawn(move || loop {
                if sync_rx.recv().is_ok() {
                    repaint_signal.request_repaint();
                }
            });

            Ok(Box::new(MyApp::new(
                ctx,
                gui_data_lock,
                gui_device_lock,
                gui_devices_lock,
                saved_serial_device_configs,
                gui_connected_lock,
                gui_settings,
                save_tx,
                load_tx,
                loaded_names_rx,
                send_tx,
                clear_tx,
            )))
        }),
    ) {
        log::error!("{e:?}");
    }
}
