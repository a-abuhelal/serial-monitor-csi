use core::f32;
use std::cmp::max;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::color_picker::{color_picker_widget, color_picker_window, COLORS};
use crate::custom_highlighter::highlight_impl;
use crate::data::{DataContainer, SerialDirection};
use crate::serial::{clear_serial_settings, save_serial_settings, Device, SerialDevices};
use crate::settings_window::settings_window;
use crate::toggle::toggle;
#[cfg(feature = "self_update")]
use crate::update::check_update;
use crate::FileOptions;
use crate::{APP_INFO, PREFERENCES_KEY};
use eframe::egui::panel::Side;
use eframe::egui::{
    Align2, CollapsingHeader, Color32, FontFamily, FontId, KeyboardShortcut, Pos2, Sense, Ui, Vec2,
};
use eframe::{egui, Storage};
use egui::ThemePreference;
use egui_file_dialog::information_panel::InformationPanel;
use egui_file_dialog::FileDialog;
use egui_plot::{log_grid_spacer, GridMark, Legend, Line, Plot, PlotPoint, PlotPoints};
use preferences::Preferences;
#[cfg(feature = "self_update")]
use self_update::update::Release;
use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, StopBits};

const DEFAULT_FONT_ID: FontId = FontId::new(14.0, FontFamily::Monospace);
pub const RIGHT_PANEL_WIDTH: f32 = 350.0;
const BAUD_RATES: &[u32] = &[
    300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 74880, 115200, 230400, 128000, 460800,
    576000, 921600,
];
const SAVE_FILE_SHORTCUT: KeyboardShortcut =
    KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);

// bitOr is not const, so we use plus
const SAVE_PLOT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(
    egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    egui::Key::S,
);

const CLEAR_PLOT_SHORTCUT: KeyboardShortcut =
    KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::X);

#[derive(Clone)]
pub enum FileDialogState {
    Open,
    Save,
    SavePlot,
    None,
}
#[derive(PartialEq)]
pub enum WindowFeedback {
    None,
    Waiting,
    Clear,
    Cancel,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct GuiSettingsContainer {
    pub device: String,
    pub baud: u32,
    pub debug: bool,
    pub x: f32,
    pub y: f32,
    pub save_absolute_time: bool,
    pub dark_mode: bool,
    pub theme_preference: ThemePreference,
}

impl Default for GuiSettingsContainer {
    fn default() -> Self {
        Self {
            device: "".to_string(),
            baud: 115_200,
            debug: true,
            x: 1600.0,
            y: 900.0,
            save_absolute_time: false,
            dark_mode: true,
            theme_preference: ThemePreference::System,
        }
    }
}

pub fn load_gui_settings() -> GuiSettingsContainer {
    GuiSettingsContainer::load(&APP_INFO, PREFERENCES_KEY).unwrap_or_else(|_| {
        let gui_settings = GuiSettingsContainer::default();
        // save default settings
        if gui_settings.save(&APP_INFO, PREFERENCES_KEY).is_err() {
            log::error!("failed to save gui_settings");
        }
        gui_settings
    })
}

pub enum ColorWindow {
    NoShow,
    ColorIndex(usize),
}

pub struct MyApp {
    connected_to_device: bool,
    command: String,
    device: String,
    old_device: String,
    device_idx: usize,
    serial_devices: SerialDevices,
    plotting_range: usize,
    plot_serial_display_ratio: f32,
    picked_path: PathBuf,
    plot_location: Option<egui::Rect>,
    data: DataContainer,
    file_dialog_state: FileDialogState,
    file_dialog: FileDialog,
    information_panel: InformationPanel,
    file_opened: bool,
    settings_window_open: bool,
    update_text: String,
    gui_conf: GuiSettingsContainer,
    device_lock: Arc<RwLock<Device>>,
    devices_lock: Arc<RwLock<Vec<String>>>,
    connected_lock: Arc<RwLock<bool>>,
    data_lock: Arc<RwLock<DataContainer>>,
    save_tx: Sender<FileOptions>,
    load_tx: Sender<PathBuf>,
    load_names_rx: Receiver<Vec<String>>,
    send_tx: Sender<String>,
    clear_tx: Sender<bool>,
    history: Vec<String>,
    index: usize,
    eol: String,
    colors: Vec<Color32>,
    color_vals: Vec<f32>,
    labels: Vec<String>,
    show_color_window: ColorWindow,
    show_sent_cmds: bool,
    show_timestamps: bool,
    save_raw: bool,
    show_warning_window: WindowFeedback,
    do_not_show_clear_warning: bool,
    init: bool,
    //new
    //set-traffic
    pub traffic_enable: bool,
    pub traffic_type: usize,
    pub traffic_interval: String,
    // Add these for the new section:
    //set-network
    pub network_architecture: usize,
    //set-wifi
    pub wifi_mode: usize,
    pub wifi_max_connections: String,
    pub wifi_hide_ssid: bool,
    pub wifi_ap_ssid: String,
    pub wifi_ap_password: String,
    pub wifi_sta_ssid: String,
    pub wifi_sta_password: String,
    //set-csi
    pub csi_disable_lltf: bool,
    pub csi_disable_htltf: bool,
    pub csi_disable_stbc_htltf: bool,
    pub csi_disable_ltf_merge: bool,
    //start button checkboxes
    pub enable_set_traffic: bool,
    pub enable_set_network: bool,
    pub enable_set_csi: bool,
    pub enable_set_wifi: bool,

    #[cfg(feature = "self_update")]
    new_release: Option<Release>,
}

#[allow(clippy::too_many_arguments)]
impl MyApp {
    pub fn new(
        cc: &eframe::CreationContext,
        data_lock: Arc<RwLock<DataContainer>>,
        device_lock: Arc<RwLock<Device>>,
        devices_lock: Arc<RwLock<Vec<String>>>,
        devices: SerialDevices,
        connected_lock: Arc<RwLock<bool>>,
        gui_conf: GuiSettingsContainer,
        save_tx: Sender<FileOptions>,
        load_tx: Sender<PathBuf>,
        load_names_rx: Receiver<Vec<String>>,
        send_tx: Sender<String>,
        clear_tx: Sender<bool>,
    ) -> Self {
        let mut file_dialog = FileDialog::default()
            //.initial_directory(PathBuf::from("/path/to/app"))
            .default_file_name("measurement.csv")
            .default_size([600.0, 400.0])
            // .add_quick_access("Project", |s| {
            //     s.add_path("☆  Examples", "examples");
            //     s.add_path("📷  Media", "media");
            //     s.add_path("📂  Source", "src");
            // })
            .set_file_icon(
                "🖹",
                Arc::new(|path| path.extension().unwrap_or_default().to_ascii_lowercase() == "md"),
            )
            .set_file_icon(
                "",
                Arc::new(|path| {
                    path.file_name().unwrap_or_default().to_ascii_lowercase() == ".gitignore"
                }),
            )
            .add_file_filter(
                "CSV files",
                Arc::new(|p| p.extension().unwrap_or_default().to_ascii_lowercase() == "csv"),
            );
        // Load the persistent data of the file dialog.
        // Alternatively, you can also use the `FileDialog::storage` builder method.
        if let Some(storage) = cc.storage {
            *file_dialog.storage_mut() =
                eframe::get_value(storage, "file_dialog_storage").unwrap_or_default()
        }

        Self {
            //new
            //set-traffic
            traffic_enable: false,
            traffic_type: 0,
            traffic_interval: String::new(),
            //set-network
            network_architecture: 3,
            //set-wifi
            wifi_mode: 2, // default to "sniffer"
            wifi_max_connections: "1".to_string(),
            wifi_hide_ssid: false,
            wifi_ap_ssid: String::new(),
            wifi_ap_password: String::new(),
            wifi_sta_ssid: String::new(),
            wifi_sta_password: String::new(),
            //set-csi
            csi_disable_lltf: false,
            csi_disable_htltf: false,
            csi_disable_stbc_htltf: false,
            csi_disable_ltf_merge: false,
            //start button checkboxes
            enable_set_traffic: false,
            enable_set_network: false,
            enable_set_csi: false,
            enable_set_wifi: false,
            //old
            connected_to_device: false,
            picked_path: PathBuf::new(),
            device: "".to_string(),
            old_device: "".to_string(),
            data: DataContainer::default(),
            file_dialog_state: FileDialogState::None,
            file_dialog,
            information_panel: InformationPanel::default().add_file_preview("csv", |ui, item| {
                ui.label("CSV preview:");
                if let Some(mut content) = item.content() {
                    egui::ScrollArea::vertical()
                        .max_height(ui.available_height())
                        .show(ui, |ui| {
                            ui.add(egui::TextEdit::multiline(&mut content).code_editor());
                        });
                }
            }),
            connected_lock,
            device_lock,
            devices_lock,
            device_idx: 0,
            serial_devices: devices,
            gui_conf,
            data_lock,
            save_tx,
            load_tx,
            load_names_rx,
            send_tx,
            clear_tx,
            plotting_range: usize::MAX,
            plot_serial_display_ratio: 0.45,
            command: "".to_string(),
            show_sent_cmds: true,
            show_timestamps: true,
            save_raw: false,
            eol: "\\r\\n".to_string(),
            colors: vec![COLORS[0]],
            color_vals: vec![0.0],
            labels: vec!["Column 0".to_string()],
            history: vec![],
            index: 0,
            plot_location: None,
            do_not_show_clear_warning: false,
            show_warning_window: WindowFeedback::None,
            init: false,
            show_color_window: ColorWindow::NoShow,
            file_opened: false,
            #[cfg(feature = "self_update")]
            new_release: None,
            settings_window_open: false,
            update_text: "".to_string(),
        }
    }

    pub fn clear_warning_window(&mut self, ctx: &egui::Context) -> WindowFeedback {
        let mut window_feedback = WindowFeedback::Waiting;
        egui::Window::new("Attention!")
            .fixed_pos(Pos2 { x: 800.0, y: 450.0 })
            .fixed_size(Vec2 { x: 400.0, y: 200.0 })
            .anchor(Align2::CENTER_CENTER, Vec2 { x: 0.0, y: 0.0 })
            .collapsible(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label("Changing devices will clear all data.");
                    ui.label("How do you want to proceed?");
                    ui.add_space(20.0);
                    ui.checkbox(&mut self.do_not_show_clear_warning, "Remember my decision.");
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.add_space(130.0);
                        if ui.button("Continue & Clear").clicked() {
                            window_feedback = WindowFeedback::Clear;
                        }
                        if ui.button("Cancel").clicked() {
                            window_feedback = WindowFeedback::Cancel;
                        }
                    });
                    ui.add_space(5.0);
                });
            });
        window_feedback
    }

    fn console_text(&self, packet: &crate::data::Packet) -> Option<String> {
        match (self.show_sent_cmds, self.show_timestamps, &packet.direction) {
            (true, true, _) => Some(format!(
                "[{}] t + {:.3}s: {}\n",
                packet.direction,
                packet.relative_time as f32 / 1000.0,
                packet.payload
            )),
            (true, false, _) => Some(format!("[{}]: {}\n", packet.direction, packet.payload)),
            (false, true, SerialDirection::Receive) => Some(format!(
                "t + {:.3}s: {}\n",
                packet.relative_time as f32 / 1000.0,
                packet.payload
            )),
            (false, false, SerialDirection::Receive) => Some(packet.payload.clone() + "\n"),
            (_, _, _) => None,
        }
    }

    fn draw_central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let left_border = 10.0;
            // Width
            let width = ui.available_size().x - 2.0 * left_border - RIGHT_PANEL_WIDTH;
            // Height
            let top_spacing = 5.0;
            let panel_height = ui.available_size().y;
            let mut plot_height: f32 = 0.0;

            if self.serial_devices.number_of_plots[self.device_idx] > 0 {
                let height = ui.available_size().y * self.plot_serial_display_ratio;
                plot_height = height;
                // need to subtract 12.0, this seems to be the height of the separator of two adjacent plots
                plot_height = plot_height
                    / (self.serial_devices.number_of_plots[self.device_idx] as f32)
                    - 12.0;
            }

            let mut plot_ui_heigh: f32 = 0.0;

            ui.add_space(top_spacing);
            ui.horizontal(|ui| {
                ui.add_space(left_border);
                ui.vertical(|ui| {
                    if let Ok(read_guard) = self.data_lock.read() {
                        self.data = read_guard.clone();
                    }

                    if self.data.loaded_from_file && self.file_opened {
                        if let Ok(labels) =
                            self.load_names_rx.recv_timeout(Duration::from_millis(10))
                        {
                            self.labels = labels;
                            self.colors = (0..max(self.labels.len(), 1))
                                .map(|i| COLORS[i % COLORS.len()])
                                .collect();
                            self.color_vals = (0..max(self.labels.len(), 1)).map(|_| 0.0).collect();
                        }
                    }
                    if self.serial_devices.number_of_plots[self.device_idx] > 0 {
                        if self.data.dataset.len() != self.labels.len() && !self.file_opened {
                            self.labels = (0..max(self.data.dataset.len(), 1))
                                .map(|i| format!("Column {i}"))
                                .collect();
                            self.colors = (0..max(self.data.dataset.len(), 1))
                                .map(|i| COLORS[i % COLORS.len()])
                                .collect();
                            self.color_vals =
                                (0..max(self.data.dataset.len(), 1)).map(|_| 0.0).collect();
                        }

                        let mut graphs: Vec<Vec<PlotPoint>> = vec![vec![]; self.data.dataset.len()];
                        let window = self.data.dataset[0]
                            .len()
                            .saturating_sub(self.plotting_range);

                        for (i, time) in self.data.time[window..].iter().enumerate() {
                            let x = *time / 1000.0;
                            for (graph, data) in graphs.iter_mut().zip(&self.data.dataset) {
                                if self.data.time.len() == data.len() {
                                    if let Some(y) = data.get(i + window) {
                                        graph.push(PlotPoint { x, y: *y as f64 });
                                    }
                                }
                            }
                        }

                        let t_fmt = |x: GridMark, _range: &RangeInclusive<f64>| {
                            format!("{:4.2} s", x.value)
                        };

                        let plots_ui = ui.vertical(|ui| {
                            for graph_idx in 0..self.serial_devices.number_of_plots[self.device_idx]
                            {
                                if graph_idx != 0 {
                                    ui.separator();
                                }

                                let signal_plot = Plot::new(format!("data-{graph_idx}"))
                                    .height(plot_height)
                                    .width(width)
                                    .legend(Legend::default())
                                    .x_grid_spacer(log_grid_spacer(10))
                                    .y_grid_spacer(log_grid_spacer(10))
                                    .x_axis_formatter(t_fmt);

                                let plot_inner = signal_plot.show(ui, |signal_plot_ui| {
                                    for (i, graph) in graphs.iter().enumerate() {
                                        // this check needs to be here for when we change devices (not very elegant)
                                        if i < self.labels.len() {
                                            signal_plot_ui.line(
                                                Line::new(PlotPoints::Owned(graph.to_vec()))
                                                    .name(&self.labels[i])
                                                    .color(self.colors[i]),
                                            );
                                        }
                                    }
                                });

                                self.plot_location = Some(plot_inner.response.rect);
                            }
                            let separator_response = ui.separator();
                            let separator = ui
                                .interact(
                                    separator_response.rect,
                                    separator_response.id,
                                    Sense::click_and_drag(),
                                )
                                .on_hover_cursor(egui::CursorIcon::ResizeVertical);

                            let resize_y = separator.drag_delta().y;

                            if separator.double_clicked() {
                                self.plot_serial_display_ratio = 0.45;
                            }
                            self.plot_serial_display_ratio = (self.plot_serial_display_ratio
                                + resize_y / panel_height)
                                .clamp(0.1, 0.9);

                            ui.add_space(top_spacing);
                        });
                        plot_ui_heigh = plots_ui.response.rect.height();
                    } else {
                        plot_ui_heigh = 0.0;
                    }

                    let serial_height =
                        panel_height - plot_ui_heigh - left_border * 2.0 - top_spacing;

                    let num_rows = self.data.raw_traffic.len();
                    let row_height = ui.text_style_height(&egui::TextStyle::Body);

                    let color = if self.gui_conf.dark_mode {
                        Color32::WHITE
                    } else {
                        Color32::BLACK
                    };

                    let mut text_edit_size = ui.available_size();
                    text_edit_size.x = width;
                    egui::ScrollArea::vertical()
                        .id_salt("serial_output")
                        .auto_shrink([false; 2])
                        .stick_to_bottom(true)
                        .enable_scrolling(true)
                        .max_height(serial_height - top_spacing)
                        .min_scrolled_height(serial_height - top_spacing)
                        .max_width(width)
                        .show_rows(ui, row_height, num_rows, |ui, row_range| {
                            let content: String = row_range
                                .into_iter()
                                .flat_map(|i| {
                                    if self.data.raw_traffic.is_empty() {
                                        None
                                    } else {
                                        self.console_text(&self.data.raw_traffic[i])
                                    }
                                })
                                .collect();

                            let mut layouter = |ui: &egui::Ui, string: &str, wrap_width: f32| {
                                let mut layout_job = highlight_impl(
                                    ui.ctx(),
                                    string,
                                    self.serial_devices.highlight_labels[self.device_idx].clone(),
                                    Color32::from_rgb(155, 164, 167),
                                )
                                .unwrap();
                                layout_job.wrap.max_width = wrap_width;
                                ui.fonts(|f| f.layout_job(layout_job))
                            };

                            ui.add(
                                egui::TextEdit::multiline(&mut content.as_str())
                                    .font(DEFAULT_FONT_ID) // for cursor height
                                    .lock_focus(true)
                                    .text_color(color)
                                    .desired_width(width)
                                    .layouter(&mut layouter),
                            );
                        });
                    ui.horizontal(|ui| {
                        let cmd_line = ui.add(
                            egui::TextEdit::singleline(&mut self.command)
                                .desired_width(width - 50.0)
                                .lock_focus(true)
                                .code_editor(),
                        );
                        let cmd_has_lost_focus = cmd_line.lost_focus();
                        let key_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (key_pressed && cmd_has_lost_focus) || ui.button("Send").clicked() {
                            // send command
                            self.history.push(self.command.clone());
                            self.index = self.history.len() - 1;
                            let eol = self.eol.replace("\\r", "\r").replace("\\n", "\n");
                            if let Err(err) = self.send_tx.send(self.command.clone() + &eol) {
                                log::error!("send_tx thread send failed: {:?}", err);
                            }
                            // stay in focus!
                            cmd_line.request_focus();
                        }
                    });

                    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                        self.index = self.index.saturating_sub(1);
                        if !self.history.is_empty() {
                            self.command = self.history[self.index].clone();
                        }
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                        self.index = std::cmp::min(self.index + 1, self.history.len() - 1);
                        if !self.history.is_empty() {
                            self.command = self.history[self.index].clone();
                        }
                    }
                });
                ui.add_space(left_border);
            });
        });
    }

    fn draw_serial_settings(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("Serial Monitor");
            self.paint_connection_indicator(ui);
        });

        let devices: Vec<String> = if let Ok(read_guard) = self.devices_lock.read() {
            read_guard.clone()
        } else {
            vec![]
        };

        if !devices.contains(&self.device) {
            self.device.clear();
        }
        if let Ok(dev) = self.device_lock.read() {
            if !dev.name.is_empty() {
                self.device = dev.name.clone();
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Device");
            ui.add_space(130.0);
            ui.label("Baud");
        });

        let old_name = self.device.clone();
        ui.horizontal(|ui| {
            if self.file_opened {
                ui.disable();
            }
            let dev_text = self.device.replace("/dev/tty.", "");
            ui.horizontal(|ui| {
                if self.connected_to_device {
                    ui.disable();
                }
                let _response = egui::ComboBox::from_id_salt("Device")
                    .selected_text(dev_text)
                    .width(RIGHT_PANEL_WIDTH * 0.92 - 155.0)
                    .show_ui(ui, |ui| {
                        devices
                            .into_iter()
                            // on macOS each device appears as /dev/tty.* and /dev/cu.*
                            // we only display the /dev/tty.* here
                            .filter(|dev| !dev.contains("/dev/cu."))
                            .for_each(|dev| {
                                // this makes the names shorter in the UI on UNIX and UNIX-like platforms
                                let dev_text = dev.replace("/dev/tty.", "");
                                ui.selectable_value(&mut self.device, dev, dev_text);
                            });
                    })
                    .response;
                // let selected_new_device = response.changed();  //somehow this does not work
                // if selected_new_device {
                if old_name != self.device {
                    if !self.data.time.is_empty() {
                        self.show_warning_window = WindowFeedback::Waiting;
                        self.old_device = old_name;
                    } else {
                        self.show_warning_window = WindowFeedback::Clear;
                    }
                }
            });
            match self.show_warning_window {
                WindowFeedback::None => {}
                WindowFeedback::Waiting => {
                    self.show_warning_window = self.clear_warning_window(ctx);
                }
                WindowFeedback::Clear => {
                    // new device selected, check in previously used devices
                    let mut device_is_already_saved = false;
                    for (idx, dev) in self.serial_devices.devices.iter().enumerate() {
                        if dev.name == self.device {
                            // this is the device!
                            self.device = dev.name.clone();
                            self.device_idx = idx;
                            self.init = true;
                            device_is_already_saved = true;
                        }
                    }
                    if !device_is_already_saved {
                        // create new device in the archive
                        let mut device = Device::default();
                        device.name = self.device.clone();
                        self.serial_devices.devices.push(device);
                        self.serial_devices.number_of_plots.push(1);
                        self.serial_devices.number_of_highlights.push(1);
                        self.serial_devices
                            .highlight_labels
                            .push(vec!["".to_string()]);
                        self.serial_devices
                            .labels
                            .push(vec!["Column 0".to_string()]);
                        self.device_idx = self.serial_devices.devices.len() - 1;
                        save_serial_settings(&self.serial_devices);
                    }
                    self.clear_tx
                        .send(true)
                        .expect("failed to send clear after choosing new device");
                    // need to clear the data here such that we don't get errors in the gui (plot)
                    self.data = DataContainer::default();
                    self.show_warning_window = WindowFeedback::None;
                }
                WindowFeedback::Cancel => {
                    self.device = self.old_device.clone();
                    self.show_warning_window = WindowFeedback::None;
                }
            }
            egui::ComboBox::from_id_salt("Baud Rate")
                .selected_text(format!(
                    "{}",
                    self.serial_devices.devices[self.device_idx].baud_rate
                ))
                .width(80.0)
                .show_ui(ui, |ui| {
                    if self.connected_to_device {
                        ui.disable();
                    }
                    BAUD_RATES.iter().for_each(|baud_rate| {
                        ui.selectable_value(
                            &mut self.serial_devices.devices[self.device_idx].baud_rate,
                            *baud_rate,
                            baud_rate.to_string(),
                        );
                    });
                });
            let connect_text = if self.connected_to_device {
                "Disconnect"
            } else {
                "Connect"
            };
            if ui.button(connect_text).clicked() {
                if let Ok(mut device) = self.device_lock.write() {
                    if self.connected_to_device {
                        let _ = self.send_tx.send("__CTRLC__\r\n".to_string()); // send Ctrl+R to the device
                        device.name.clear();
                    } else {
                        device.name = self.serial_devices.devices[self.device_idx].name.clone();
                        device.baud_rate = self.serial_devices.devices[self.device_idx].baud_rate;
                        let _ = self.send_tx.send("__RESET__\r\n".to_string()); // send Ctrl+R to the device
                    }
                }
            }
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Data Bits");
            ui.add_space(5.0);
            ui.label("Parity");
            ui.add_space(20.0);
            ui.label("Stop Bits");
            ui.label("Flow Control");
            ui.label("Timeout");
        });
        ui.horizontal(|ui| {
            if self.connected_to_device {
                ui.disable();
            }
            egui::ComboBox::from_id_salt("Data Bits")
                .selected_text(
                    self.serial_devices.devices[self.device_idx]
                        .data_bits
                        .to_string(),
                )
                .width(30.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].data_bits,
                        DataBits::Eight,
                        DataBits::Eight.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].data_bits,
                        DataBits::Seven,
                        DataBits::Seven.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].data_bits,
                        DataBits::Six,
                        DataBits::Six.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].data_bits,
                        DataBits::Five,
                        DataBits::Five.to_string(),
                    );
                });
            egui::ComboBox::from_id_salt("Parity")
                .selected_text(
                    self.serial_devices.devices[self.device_idx]
                        .parity
                        .to_string(),
                )
                .width(30.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].parity,
                        Parity::None,
                        Parity::None.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].parity,
                        Parity::Odd,
                        Parity::Odd.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].parity,
                        Parity::Even,
                        Parity::Even.to_string(),
                    );
                });
            egui::ComboBox::from_id_salt("Stop Bits")
                .selected_text(
                    self.serial_devices.devices[self.device_idx]
                        .stop_bits
                        .to_string(),
                )
                .width(30.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].stop_bits,
                        StopBits::One,
                        StopBits::One.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].stop_bits,
                        StopBits::Two,
                        StopBits::Two.to_string(),
                    );
                });
            egui::ComboBox::from_id_salt("Flow Control")
                .selected_text(
                    self.serial_devices.devices[self.device_idx]
                        .flow_control
                        .to_string(),
                )
                .width(75.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].flow_control,
                        FlowControl::None,
                        FlowControl::None.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].flow_control,
                        FlowControl::Hardware,
                        FlowControl::Hardware.to_string(),
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].flow_control,
                        FlowControl::Software,
                        FlowControl::Software.to_string(),
                    );
                });
            egui::ComboBox::from_id_salt("Timeout")
                .selected_text(
                    self.serial_devices.devices[self.device_idx]
                        .timeout
                        .as_millis()
                        .to_string(),
                )
                .width(55.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].timeout,
                        Duration::from_millis(0),
                        "0",
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].timeout,
                        Duration::from_millis(10),
                        "10",
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].timeout,
                        Duration::from_millis(100),
                        "100",
                    );
                    ui.selectable_value(
                        &mut self.serial_devices.devices[self.device_idx].timeout,
                        Duration::from_millis(1000),
                        "1000",
                    );
                });
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            if self.connected_to_device {
                ui.disable();
            }
            if ui
                .button(egui::RichText::new(format!(
                    "{} Open file",
                    egui_phosphor::regular::FOLDER_OPEN
                )))
                .on_hover_text("Load data from .csv")
                .clicked()
            {
                self.file_dialog_state = FileDialogState::Open;
                self.file_dialog.pick_file();
            }
            if self.file_opened
                && ui
                    .button(egui::RichText::new(
                        egui_phosphor::regular::X_SQUARE.to_string(),
                    ))
                    .on_hover_text("Close file.")
                    .clicked()
            {
                self.file_opened = false;
                let _ = self.load_tx.send(PathBuf::new());
                self.file_dialog_state = FileDialogState::None;
            }
        });
    }
    fn draw_export_settings(&mut self, _ctx: &egui::Context, ui: &mut Ui) {
        egui::Grid::new("export_settings")
            .num_columns(2)
            .spacing(Vec2 { x: 10.0, y: 10.0 })
            .striped(true)
            .show(ui, |ui| {
                if ui
                    .button(egui::RichText::new(format!(
                        "{} Save CSV",
                        egui_phosphor::regular::FLOPPY_DISK
                    )))
                    .on_hover_text("Save Plot Data to CSV.")
                    .clicked()
                    || ui.input_mut(|i| i.consume_shortcut(&SAVE_FILE_SHORTCUT))
                {
                    self.file_dialog_state = FileDialogState::Save;
                    self.file_dialog.save_file();
                }

                if ui
                    .button(egui::RichText::new(format!(
                        "{} Save Plot",
                        egui_phosphor::regular::FLOPPY_DISK
                    )))
                    .on_hover_text("Save an image of the Plot.")
                    .clicked()
                    || ui.input_mut(|i| i.consume_shortcut(&SAVE_PLOT_SHORTCUT))
                {
                    self.file_dialog_state = FileDialogState::SavePlot;
                    self.file_dialog.save_file();
                }
                ui.end_row();
                ui.label("Save Raw Traffic");
                ui.add(toggle(&mut self.save_raw))
                    .on_hover_text("Save second CSV containing raw traffic.")
                    .changed();
                ui.end_row();
                ui.label("Save Absolute Time");
                ui.add(toggle(&mut self.gui_conf.save_absolute_time))
                    .on_hover_text("Save absolute time in CSV.");
                ui.end_row();
            });
    }

    fn draw_global_settings(&mut self, ui: &mut Ui) {
        ui.add_space(20.0);

        if ui
            .button(format!("{} Settings", egui_phosphor::regular::GEAR_FINE))
            .clicked()
        {
            #[cfg(feature = "self_update")]
            {
                self.new_release = check_update();
            }
            self.settings_window_open = true;
        }
        if self.settings_window_open {
            settings_window(
                ui.ctx(),
                &mut self.gui_conf,
                #[cfg(feature = "self_update")]
                &mut self.new_release,
                &mut self.settings_window_open,
                &mut self.update_text,
            );
        }

        ui.add_space(20.0);

        if ui
            .button(egui::RichText::new(format!(
                "{} Clear Data",
                egui_phosphor::regular::X
            )))
            .on_hover_text("Clear Data from Plot.")
            .clicked()
            || ui.input_mut(|i| i.consume_shortcut(&CLEAR_PLOT_SHORTCUT))
        {
            log::info!("Cleared recorded Data");
            if let Err(err) = self.clear_tx.send(true) {
                log::error!("clear_tx thread send failed: {:?}", err);
            }
            // need to clear the data here in order to prevent errors in the gui (plot)
            self.data = DataContainer::default();
            // self.names_tx.send(self.serial_devices.labels[self.device_idx].clone()).expect("Failed to send names");
        }
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            if ui.button("Clear Device History").clicked() {
                self.serial_devices = SerialDevices::default();
                self.device.clear();
                self.device_idx = 0;
                clear_serial_settings();
            }
            if ui.button("Reset Labels").clicked() {
                // self.serial_devices.labels[self.device_idx] = self.serial_devices.labels.clone();
            }
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.add(toggle(&mut self.show_sent_cmds))
                .on_hover_text("Show sent commands in console.");
            ui.label("Show Sent Commands");
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.add(toggle(&mut self.show_timestamps))
                .on_hover_text("Show timestamp in console.");
            ui.label("Show Timestamp");
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("EOL character");
            ui.add(
                egui::TextEdit::singleline(&mut self.eol).desired_width(ui.available_width() * 0.9),
            )
            .on_hover_text("Configure your EOL character for sent commands..");
        });
    }

    fn draw_plot_settings(&mut self, ui: &mut Ui) {
        egui::Grid::new("plot_settings")
            .num_columns(2)
            .spacing(Vec2 { x: 10.0, y: 10.0 })
            .striped(true)
            .show(ui, |ui| {
                ui.label("Plotting range [#]: ");

                let window_fmt = |val: f64, _range: RangeInclusive<usize>| {
                    if val != usize::MAX as f64 {
                        val.to_string()
                    } else {
                        "∞".to_string()
                    }
                };

                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.plotting_range).custom_formatter(window_fmt),
                    )
                    .on_hover_text(
                        "Select a window of the last datapoints to be displayed in the plot.",
                    );
                    if ui
                        .button("Full Dataset")
                        .on_hover_text("Show the full dataset.")
                        .clicked()
                    {
                        self.plotting_range = usize::MAX;
                    }
                });
                ui.end_row();
                ui.label("Number of plots [#]: ");

                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new(
                            egui_phosphor::regular::ARROW_FAT_LEFT.to_string(),
                        ))
                        .clicked()
                    {
                        if self.serial_devices.number_of_plots[self.device_idx] == 0 {
                            return;
                        }
                        self.serial_devices.number_of_plots[self.device_idx] =
                            (self.serial_devices.number_of_plots[self.device_idx] - 1).clamp(0, 10);
                    }
                    ui.add(
                        egui::DragValue::new(
                            &mut self.serial_devices.number_of_plots[self.device_idx],
                        )
                        .range(0..=10),
                    )
                    .on_hover_text("Select the number of plots to be shown.");
                    if ui
                        .button(egui::RichText::new(
                            egui_phosphor::regular::ARROW_FAT_RIGHT.to_string(),
                        ))
                        .clicked()
                    {
                        if self.serial_devices.number_of_plots[self.device_idx] == 10 {
                            return;
                        }
                        self.serial_devices.number_of_plots[self.device_idx] =
                            (self.serial_devices.number_of_plots[self.device_idx] + 1).clamp(0, 10);
                    }
                });
                ui.end_row();
            });
        ui.add_space(25.0);

        if self.labels.len() == 1 {
            ui.label("Detected 1 Dataset:");
        } else {
            ui.label(format!("Detected {} Datasets:", self.labels.len()));
        }
        ui.add_space(5.0);
        for i in 0..self.labels.len().min(10) {
            // if init, set names to what has been stored in the device last time
            if self.init {
                // self.names_tx.send(self.labels.clone()).expect("Failed to send names");
                self.init = false;
            }

            if self.labels.len() <= i {
                break;
            }
            ui.horizontal(|ui| {
                let response = color_picker_widget(ui, "", &mut self.colors, i);

                // Check if the square was clicked and toggle color picker window
                if response.clicked() {
                    self.show_color_window = ColorWindow::ColorIndex(i);
                };

                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.labels[i])
                            .desired_width(0.95 * RIGHT_PANEL_WIDTH),
                    )
                    .on_hover_text("Use custom names for your Datasets.")
                    .changed()
                {
                    // self.names_tx.send(self.labels.clone()).expect("Failed to send names");
                };
            });
        }
        match self.show_color_window {
            ColorWindow::NoShow => {}
            ColorWindow::ColorIndex(index) => {
                if color_picker_window(
                    ui.ctx(),
                    &mut self.colors[index],
                    &mut self.color_vals[index],
                ) {
                    self.show_color_window = ColorWindow::NoShow;
                }
            }
        }

        if self.labels.len() > 10 {
            ui.label("Only renaming up to 10 Datasets is currently supported.");
        }
    }

    fn draw_highlight_settings(&mut self, _ctx: &egui::Context, ui: &mut Ui) {
        egui::Grid::new("highlight_settings")
            .num_columns(2)
            .spacing(Vec2 { x: 10.0, y: 10.0 })
            .striped(true)
            .show(ui, |ui| {
                ui.label("Number of sentence [#]: ");

                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new(
                            egui_phosphor::regular::ARROW_FAT_LEFT.to_string(),
                        ))
                        .clicked()
                    {
                        self.serial_devices.number_of_highlights[self.device_idx] =
                            (self.serial_devices.number_of_highlights[self.device_idx] - 1)
                                .clamp(1, 4);
                        while self.serial_devices.number_of_highlights[self.device_idx]
                            < self.serial_devices.highlight_labels[self.device_idx].len()
                        {
                            self.serial_devices.highlight_labels[self.device_idx].truncate(
                                self.serial_devices.number_of_highlights[self.device_idx],
                            );
                        }
                    }
                    ui.add(
                        egui::DragValue::new(
                            &mut self.serial_devices.number_of_highlights[self.device_idx],
                        )
                        .range(1..=4),
                    )
                    .on_hover_text("Select the number of sentence to be highlighted.");
                    if ui
                        .button(egui::RichText::new(
                            egui_phosphor::regular::ARROW_FAT_RIGHT.to_string(),
                        ))
                        .clicked()
                    {
                        self.serial_devices.number_of_highlights[self.device_idx] =
                            (self.serial_devices.number_of_highlights[self.device_idx] + 1)
                                .clamp(1, 4);
                    }
                    while self.serial_devices.number_of_highlights[self.device_idx]
                        > self.serial_devices.highlight_labels[self.device_idx].len()
                    {
                        self.serial_devices.highlight_labels[self.device_idx].push("".to_string());
                    }
                });
            });

        ui.label(format!(
            "Detected {} highlight:",
            self.serial_devices.number_of_highlights[self.device_idx]
        ));

        ui.add_space(5.0);
        for i in 0..(self.serial_devices.number_of_highlights[self.device_idx]) {
            ui.add(
                egui::TextEdit::singleline(
                    &mut self.serial_devices.highlight_labels[self.device_idx][i],
                )
                .desired_width(0.95 * RIGHT_PANEL_WIDTH),
            )
            .on_hover_text("Sentence to highlight");
            /*
            // Todo implement the color picker for each sentence
            let mut theme =
                egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
            ui.collapsing(self.serial_devices.highlight_labels[self.device_idx][i].clone(), |ui| {
                theme.ui(ui);
                theme.store_in_memory(ui.ctx());
            });
            */
        }
    }

    fn draw_side_panel(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::new(Side::Right, "settings panel")
            .min_width(RIGHT_PANEL_WIDTH)
            .max_width(RIGHT_PANEL_WIDTH)
            .resizable(false)
            //.default_width(right_panel_width)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(true, |ui| {
                        self.draw_serial_settings(ctx, ui);

                        self.draw_global_settings(ui);
                        ui.add_space(10.0);
                    // Add this to your update function or wherever you build your right panel UI
                        // --- Start Section ---
                        // ui.add_space(16.0);
                        // if ui.add_sized([RIGHT_PANEL_WIDTH * 0.9, 40.0], egui::Button::new("START")).clicked() {
                        //     let _ = self.send_tx.send("start\r\n".to_string());
                        // }
                        
                        ui.add_space(16.0);
                        if ui.add_sized([RIGHT_PANEL_WIDTH * 0.9, 40.0], egui::Button::new("START")).clicked() {
                            use std::{thread, time::Duration};

                            // 1. set-traffic (only if enabled in section)
                            // if self.enable_set_traffic && self.traffic_enable {
                            //     let mut cmd = String::from("set-traffic --enable");
                            //     cmd.push_str(&format!(" --type={}", match self.traffic_type {
                            //         0 => "icmp",
                            //         1 => "udp",
                            //         _ => "unknown",
                            //     }));
                            //     if !self.traffic_interval.trim().is_empty() {
                            //         cmd.push_str(&format!(" --interval={}", self.traffic_interval.trim()));
                            //     }
                            //     cmd.push_str("\r\n");
                            //     let _ = self.send_tx.send(cmd);
                            //     thread::sleep(Duration::from_millis(150));
                            // }
                            if self.traffic_enable {
                            let mut cmd = String::from("set-traffic --enable");
                            cmd.push_str(&format!(" --type={}", match self.traffic_type {
                                0 => "icmp",
                                1 => "udp",
                                _ => "unknown",
                            }));
                            if !self.traffic_interval.trim().is_empty() {
                                cmd.push_str(&format!(" --interval={}", self.traffic_interval.trim()));
                            }
                            cmd.push_str("\r\n");
                            let _ = self.send_tx.send(cmd);
                            thread::sleep(Duration::from_millis(150));
                        }

                            // 2. set-network (only if enabled in section)
                            // if self.enable_set_network {
                            //     let arch = match self.network_architecture {
                            //         0 => "rsta",
                            //         1 => "rapsta",
                            //         2 => "apsta",
                            //         3 => "sniff",
                            //         _ => "",
                            //     };
                            //     if !arch.is_empty() {
                            //         let cmd = format!("set-network --arch={}\r\n", arch);
                            //         let _ = self.send_tx.send(cmd);
                            //         thread::sleep(Duration::from_millis(150));
                            //     }
                            // }
                            // 2. set-network (always send)
                            let arch = match self.network_architecture {
                                0 => "rsta",
                                1 => "rapsta",
                                2 => "apsta",
                                3 => "sniff",
                                _ => "",
                            };
                            if !arch.is_empty() {
                                let cmd = format!("set-network --arch={}\r\n", arch);
                                let _ = self.send_tx.send(cmd);
                                thread::sleep(Duration::from_millis(150));
                            }

                            // // 3. set-csi (only if enabled in section)
                            // if self.enable_set_csi && (self.csi_disable_lltf || self.csi_disable_htltf || self.csi_disable_stbc_htltf || self.csi_disable_ltf_merge) {
                            //     let mut cmd = String::from("set-csi");
                            //     if self.csi_disable_lltf {
                            //         cmd.push_str(" --disable-lltf");
                            //     }
                            //     if self.csi_disable_htltf {
                            //         cmd.push_str(" --disable-htltf");
                            //     }
                            //     if self.csi_disable_stbc_htltf {
                            //         cmd.push_str(" --disable-stbc-htltf");
                            //     }
                            //     if self.csi_disable_ltf_merge {
                            //         cmd.push_str(" --disable-ltf-merge");
                            //     }
                            //     cmd.push_str("\r\n");
                            //     let _ = self.send_tx.send(cmd);
                            //     thread::sleep(Duration::from_millis(150));
                            // }

                            // 3. set-csi (send only if at least one option is checked)
                            if self.csi_disable_lltf || self.csi_disable_htltf || self.csi_disable_stbc_htltf || self.csi_disable_ltf_merge {
                                let mut cmd = String::from("set-csi");
                                if self.csi_disable_lltf {
                                    cmd.push_str(" --disable-lltf");
                                }
                                if self.csi_disable_htltf {
                                    cmd.push_str(" --disable-htltf");
                                }
                                if self.csi_disable_stbc_htltf {
                                    cmd.push_str(" --disable-stbc-htltf");
                                }
                                if self.csi_disable_ltf_merge {
                                    cmd.push_str(" --disable-ltf-merge");
                                }
                                cmd.push_str("\r\n");
                                let _ = self.send_tx.send(cmd);
                                thread::sleep(Duration::from_millis(150));
                            }

                            // 4. set-wifi (only if enabled in section)
                            if self.enable_set_wifi {
                                use std::{thread, time::Duration};
                                let mode_str = match self.wifi_mode {
                                    0 => "ap",
                                    1 => "station",
                                    2 => "sniffer",
                                    3 => "ap-station",
                                    _ => "sniffer",
                                };

                                // Always send mode first
                                let cmd = format!("set-wifi --mode={}\r\n", mode_str);
                                let _ = self.send_tx.send(cmd);
                                thread::sleep(Duration::from_millis(150));

                                // Only send if not sniffer
                                if self.wifi_mode != 2 {
                                    if !self.wifi_max_connections.trim().is_empty() {
                                        let cmd = format!("set-wifi --max-connections={}\r\n", self.wifi_max_connections.trim());
                                        let _ = self.send_tx.send(cmd);
                                        thread::sleep(Duration::from_millis(150));
                                    }
                                    if self.wifi_hide_ssid {
                                        let cmd = "set-wifi --hide-ssid\r\n".to_string();
                                        let _ = self.send_tx.send(cmd);
                                        thread::sleep(Duration::from_millis(150));
                                    }
                                }

                                // AP options
                                if self.wifi_mode == 0 || self.wifi_mode == 3 {
                                    if !self.wifi_ap_ssid.trim().is_empty() {
                                        let cmd = format!("set-wifi --ap-ssid={}\r\n", self.wifi_ap_ssid.trim());
                                        let _ = self.send_tx.send(cmd);
                                        thread::sleep(Duration::from_millis(150));
                                    }
                                    if !self.wifi_ap_password.trim().is_empty() {
                                        let cmd = format!("set-wifi --ap-password={}\r\n", self.wifi_ap_password.trim());
                                        let _ = self.send_tx.send(cmd);
                                        thread::sleep(Duration::from_millis(150));
                                    }
                                }

                                // STA options
                                if self.wifi_mode == 1 || self.wifi_mode == 3 {
                                    if !self.wifi_sta_ssid.trim().is_empty() {
                                        let cmd = format!("set-wifi --sta-ssid={}\r\n", self.wifi_sta_ssid.trim());
                                        let _ = self.send_tx.send(cmd);
                                        thread::sleep(Duration::from_millis(150));
                                    }
                                    if !self.wifi_sta_password.trim().is_empty() {
                                        let cmd = format!("set-wifi --sta-password={}\r\n", self.wifi_sta_password.trim());
                                        let _ = self.send_tx.send(cmd);
                                        thread::sleep(Duration::from_millis(150));
                                    }
                                }
                            }

                            // Optionally, send "start" at the end
                            let _ = self.send_tx.send("start\r\n".to_string());
                        }


                        // --- Set Traffic Section ---
                        ui.vertical(|ui| {
                        //     ui.heading("Set Traffic");
                        //     ui.checkbox(&mut self.enable_set_traffic, "Enable Set Traffic");
                        //     ui.checkbox(&mut self.traffic_enable, "Traffic Enable");
                        //     ui.horizontal(|ui| {
                        //         ui.label("Traffic Type:");
                        //         egui::ComboBox::from_id_source("traffic_type_combo")
                        //             .selected_text(match self.traffic_type {
                        //                 0 => "icmp",
                        //                 1 => "UDP",
                        //                 _ => "Unknown",
                        //             })
                        //             .show_ui(ui, |ui| {
                        //                 ui.selectable_value(&mut self.traffic_type, 0, "icmp");
                        //                 ui.selectable_value(&mut self.traffic_type, 1, "UDP");
                        //             });
                        //     });
                        //     ui.horizontal(|ui| {
                        //         ui.label("Interval (ms):");
                        //         ui.text_edit_singleline(&mut self.traffic_interval);
                        //     });
                        // if ui.button("Set Traffic").clicked() && self.enable_set_traffic {
                        //     let mut cmd = String::from("set-traffic");
                        //     if self.traffic_enable {
                        //         cmd.push_str(" --enable");
                        //     }
                        //     cmd.push_str(&format!(" --type={}", match self.traffic_type {
                        //         0 => "icmp",
                        //         1 => "udp",
                        //         _ => "unknown",
                        //     }));
                        //     if !self.traffic_interval.trim().is_empty() {
                        //         cmd.push_str(&format!(" --interval={}", self.traffic_interval.trim()));
                        //     }
                        //     cmd.push_str("\r\n");
                        //     let _ = self.send_tx.send(cmd);
                        // }
                        // --- Set Traffic Section ---
                        ui.heading("Set Traffic");
                        ui.checkbox(&mut self.traffic_enable, "Enable Traffic");
                        ui.add_enabled_ui(self.traffic_enable, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Traffic Type:");
                                egui::ComboBox::from_id_source("traffic_type_combo")
                                    .selected_text(match self.traffic_type {
                                        0 => "icmp",
                                        1 => "UDP",
                                        _ => "Unknown",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.traffic_type, 0, "icmp");
                                        ui.selectable_value(&mut self.traffic_type, 1, "UDP");
                                    });
                            });
                            ui.horizontal(|ui| {
                                ui.label("Interval (ms):");
                                ui.text_edit_singleline(&mut self.traffic_interval);
                            });
                        });

                            ui.add_space(16.0); // Space between sections

                            // --- Set Network Section ---
                            ui.heading("Set Network");
                            //ui.checkbox(&mut self.enable_set_network, "Enable Set Network");
                            ui.horizontal(|ui| {
                                ui.label("Architecture:");
                                egui::ComboBox::from_id_source("architecture_combo")
                                    .selected_text(match self.network_architecture {
                                        0 => "rsta",
                                        1 => "rapsta",
                                        2 => "apsta",
                                        3 => "sniff",
                                        _ => "Unknown",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.network_architecture, 0, "rsta");
                                        ui.selectable_value(&mut self.network_architecture, 1, "rapsta");
                                        ui.selectable_value(&mut self.network_architecture, 2, "apsta");
                                        ui.selectable_value(&mut self.network_architecture, 3, "sniff");
                                    });
                            });
                            if ui.button("Set Network").clicked() && self.enable_set_network {
                                let arch = match self.network_architecture {
                                    0 => "rsta",
                                    1 => "rapsta",
                                    2 => "apsta",
                                    3 => "sniff",
                                    _ => "unknown",
                                };
                                let cmd = format!("set-network --arch={}\r\n", arch);
                                let _ = self.send_tx.send(cmd);
                            }
                        });
                        ui.add_space(16.0); // Space between sections

                    //     // --- Set CSI Section ---
                    //     ui.heading("Set CSI");

                    //     ui.horizontal(|ui| {
                    //     if ui.button("Disable LLTF").clicked() {
                    //         // TODO: handle --disable-lltf
                    //     }
                    //     if ui.button("Disable HTLTF").clicked() {
                    //         // TODO: handle --disable-htltf
                    //     }
                    // });
                    //     ui.horizontal(|ui| {
                    //         if ui.button("Disable STBC HTLTF").clicked() {
                    //             // TODO: handle --disable-stbc-htltf
                    //         }
                    //         if ui.button("Disable LTF Merge").clicked() {
                    //             // TODO: handle --disable-ltf-merge
                    //         }
                    //     });

                    // --- Set CSI Section ---
                    // ui.heading("Set CSI");
                    // ui.checkbox(&mut self.enable_set_csi, "Enable Set CSI");
                    // ui.checkbox(&mut self.csi_disable_lltf, "Disable LLTF");
                    // ui.checkbox(&mut self.csi_disable_htltf, "Disable HTLTF");
                    // ui.checkbox(&mut self.csi_disable_stbc_htltf, "Disable STBC HTLTF");
                    // ui.checkbox(&mut self.csi_disable_ltf_merge, "Disable LTF Merge");
                    ui.heading("Set CSI");
                    ui.checkbox(&mut self.csi_disable_lltf, "Disable LLTF");
                    ui.checkbox(&mut self.csi_disable_htltf, "Disable HTLTF");
                    ui.checkbox(&mut self.csi_disable_stbc_htltf, "Disable STBC HTLTF");
                    ui.checkbox(&mut self.csi_disable_ltf_merge, "Disable LTF Merge");
                    if ui.button("Set CSI").clicked() && self.enable_set_csi {
                        let mut cmd = String::from("set-csi");
                        if self.csi_disable_lltf {
                            cmd.push_str(" --disable-lltf");
                        }
                        if self.csi_disable_htltf {
                            cmd.push_str(" --disable-htltf");
                        }
                        if self.csi_disable_stbc_htltf {
                            cmd.push_str(" --disable-stbc-htltf");
                        }
                        if self.csi_disable_ltf_merge {
                            cmd.push_str(" --disable-ltf-merge");
                        }
                        cmd.push_str("\r\n");
                        let _ = self.send_tx.send(cmd);
                    }
                        
                        ui.add_space(16.0);

                        // --- Set WiFi Section ---
                        // ui.heading("Set WiFi");
                        // ui.checkbox(&mut self.enable_set_wifi, "Enable Set WiFi");
                        // ui.horizontal(|ui| {
                        //     ui.label("Mode:");
                        //     egui::ComboBox::from_id_source("wifi_mode_combo")
                        //         .selected_text(match self.wifi_mode {
                        //             0 => "ap",
                        //             1 => "station",
                        //             2 => "sniffer",
                        //             3 => "ap-station",
                        //             _ => "sniffer",
                        //         })
                        //         .show_ui(ui, |ui| {
                        //             ui.selectable_value(&mut self.wifi_mode, 0, "ap");
                        //             ui.selectable_value(&mut self.wifi_mode, 1, "station");
                        //             ui.selectable_value(&mut self.wifi_mode, 2, "sniffer");
                        //             ui.selectable_value(&mut self.wifi_mode, 3, "ap-station");
                        //         });
                        // });
                        // ui.horizontal(|ui| {
                        //     ui.label("Max Connections:");
                        //     ui.text_edit_singleline(&mut self.wifi_max_connections);
                        // });
                        // ui.checkbox(&mut self.wifi_hide_ssid, "Hide SSID (AP only)");
                        // ui.horizontal(|ui| {
                        //     ui.label("AP SSID:");
                        //     ui.text_edit_singleline(&mut self.wifi_ap_ssid);
                        // });
                        // ui.horizontal(|ui| {
                        //     ui.label("AP Password:");
                        //     ui.text_edit_singleline(&mut self.wifi_ap_password);
                        // });
                        // ui.horizontal(|ui| {
                        //     ui.label("STA SSID:");
                        //     ui.text_edit_singleline(&mut self.wifi_sta_ssid);
                        // });
                        // ui.horizontal(|ui| {
                        //     ui.label("STA Password:");
                        //     ui.text_edit_singleline(&mut self.wifi_sta_password);
                        // });
                        // if ui.button("Set WiFi").clicked() && self.enable_set_wifi {
                        //     let mut cmd = String::from("set-wifi");
                        //     cmd.push_str(&format!(" --mode={}", match self.wifi_mode {
                        //         0 => "ap",
                        //         1 => "station",
                        //         2 => "sniffer",
                        //         3 => "ap-station",
                        //         _ => "sniffer",
                        //     }));
                        //     if !self.wifi_max_connections.trim().is_empty() {
                        //         cmd.push_str(&format!(" --max-connections={}", self.wifi_max_connections.trim()));
                        //     }
                        //     if self.wifi_hide_ssid {
                        //         cmd.push_str(" --hide-ssid");
                        //     }
                        //     if !self.wifi_ap_ssid.trim().is_empty() {
                        //         cmd.push_str(&format!(" --ap-ssid={}", self.wifi_ap_ssid.trim()));
                        //     }
                        //     if !self.wifi_ap_password.trim().is_empty() {
                        //         cmd.push_str(&format!(" --ap-password={}", self.wifi_ap_password.trim()));
                        //     }
                        //     if !self.wifi_sta_ssid.trim().is_empty() {
                        //         cmd.push_str(&format!(" --sta-ssid={}", self.wifi_sta_ssid.trim()));
                        //     }
                        //     if !self.wifi_sta_password.trim().is_empty() {
                        //         cmd.push_str(&format!(" --sta-password={}", self.wifi_sta_password.trim()));
                        //     }
                        //     cmd.push_str("\r\n");
                        //     let _ = self.send_tx.send(cmd);
                        // }

                        ui.heading("Set WiFi");
                        ui.checkbox(&mut self.enable_set_wifi, "Enable Set WiFi");

                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            egui::ComboBox::from_id_source("wifi_mode_combo")
                                .selected_text(match self.wifi_mode {
                                    0 => "ap",
                                    1 => "station",
                                    2 => "sniffer",
                                    3 => "ap-station",
                                    _ => "sniffer",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.wifi_mode, 0, "ap");
                                    ui.selectable_value(&mut self.wifi_mode, 1, "station");
                                    ui.selectable_value(&mut self.wifi_mode, 2, "sniffer");
                                    ui.selectable_value(&mut self.wifi_mode, 3, "ap-station");
                                });
                        });

                        // Logic for graying out and clearing fields
                        let is_sniffer = self.wifi_mode == 2;
                        let is_station = self.wifi_mode == 1;
                        let is_ap = self.wifi_mode == 0;
                        let is_apsta = self.wifi_mode == 3;

                        if is_sniffer {
                            self.wifi_max_connections.clear();
                            self.wifi_hide_ssid = false;
                            self.wifi_ap_ssid.clear();
                            self.wifi_ap_password.clear();
                            self.wifi_sta_ssid.clear();
                            self.wifi_sta_password.clear();
                        }
                        if is_station {
                            self.wifi_ap_ssid.clear();
                            self.wifi_ap_password.clear();
                        }
                        if is_ap {
                            self.wifi_sta_ssid.clear();
                            self.wifi_sta_password.clear();
                        }

                        // Max Connections
                        ui.horizontal(|ui| {
                            ui.label("Max Connections:");
                            ui.add_enabled_ui(!is_sniffer, |ui| {
                                ui.text_edit_singleline(&mut self.wifi_max_connections);
                            });
                        });

                        // Hide SSID
                        ui.add_enabled_ui(!is_sniffer, |ui| {
                            ui.checkbox(&mut self.wifi_hide_ssid, "Hide SSID (AP only)");
                        });

                        // AP SSID and Password
                        ui.horizontal(|ui| {
                            ui.label("AP SSID:");
                            ui.add_enabled_ui(is_ap || is_apsta, |ui| {
                                ui.text_edit_singleline(&mut self.wifi_ap_ssid);
                            });
                        });
                        ui.horizontal(|ui| {
                            ui.label("AP Password:");
                            ui.add_enabled_ui(is_ap || is_apsta, |ui| {
                                ui.text_edit_singleline(&mut self.wifi_ap_password);
                            });
                        });

                        // STA SSID and Password
                        ui.horizontal(|ui| {
                            ui.label("STA SSID:");
                            ui.add_enabled_ui(is_station || is_apsta, |ui| {
                                ui.text_edit_singleline(&mut self.wifi_sta_ssid);
                            });
                        });
                        ui.horizontal(|ui| {
                            ui.label("STA Password:");
                            ui.add_enabled_ui(is_station || is_apsta, |ui| {
                                ui.text_edit_singleline(&mut self.wifi_sta_password);
                            });
                        });

                        if ui.button("Set WiFi").clicked() && self.enable_set_wifi {
                            use std::{thread, time::Duration};
                            let mode_str = match self.wifi_mode {
                                0 => "ap",
                                1 => "station",
                                2 => "sniffer",
                                3 => "ap-station",
                                _ => "sniffer",
                            };

                            // Always send mode first
                            let cmd = format!("set-wifi --mode={}\r\n", mode_str);
                            let _ = self.send_tx.send(cmd);
                            thread::sleep(Duration::from_millis(150));

                            // Only send if not sniffer
                            if self.wifi_mode != 2 {
                                if !self.wifi_max_connections.trim().is_empty() {
                                    let cmd = format!("set-wifi --max-connections={}\r\n", self.wifi_max_connections.trim());
                                    let _ = self.send_tx.send(cmd);
                                    thread::sleep(Duration::from_millis(150));
                                }
                                if self.wifi_hide_ssid {
                                    let cmd = "set-wifi --hide-ssid\r\n".to_string();
                                    let _ = self.send_tx.send(cmd);
                                    thread::sleep(Duration::from_millis(150));
                                }
                            }

                            // AP options
                            if self.wifi_mode == 0 || self.wifi_mode == 3 {
                                if !self.wifi_ap_ssid.trim().is_empty() {
                                    let cmd = format!("set-wifi --ap-ssid={}\r\n", self.wifi_ap_ssid.trim());
                                    let _ = self.send_tx.send(cmd);
                                    thread::sleep(Duration::from_millis(150));
                                }
                                if !self.wifi_ap_password.trim().is_empty() {
                                    let cmd = format!("set-wifi --ap-password={}\r\n", self.wifi_ap_password.trim());
                                    let _ = self.send_tx.send(cmd);
                                    thread::sleep(Duration::from_millis(150));
                                }
                            }

                            // STA options
                            if self.wifi_mode == 1 || self.wifi_mode == 3 {
                                if !self.wifi_sta_ssid.trim().is_empty() {
                                    let cmd = format!("set-wifi --sta-ssid={}\r\n", self.wifi_sta_ssid.trim());
                                    let _ = self.send_tx.send(cmd);
                                    thread::sleep(Duration::from_millis(150));
                                }
                                if !self.wifi_sta_password.trim().is_empty() {
                                    let cmd = format!("set-wifi --sta-password={}\r\n", self.wifi_sta_password.trim());
                                    let _ = self.send_tx.send(cmd);
                                    thread::sleep(Duration::from_millis(150));
                                }
                            }
                        }
                        
                        //this does not work yet
                        // --- Ctrl R Section ---
                        ui.add_space(16.0);
                        ui.heading("Ctrl R");
                        if ui.button("Send Ctrl R").clicked() {
                            let _ = self.send_tx.send("__RESET__\r\n".to_string()); // send Ctrl+R to the device
                        }

                        
                        // --- Show Config Section ---
                        ui.add_space(16.0);
                        ui.heading("Show Config");
                        if ui.button("Show Config").clicked() {
                            let _ = self.send_tx.send("show-config\r\n".to_string());
                        }

                        ui.add_space(16.0);
                        ui.heading("Rest Config");
                        if ui.button("Reset Config").clicked() {
                            let _ = self.send_tx.send("reset-config\r\n".to_string());
                        }
                        
                        

                        CollapsingHeader::new("Plot Settings")
                            .default_open(true)
                            .show(ui, |ui| {
                                self.draw_plot_settings(ui);
                            });

                        CollapsingHeader::new("Text Highlight Settings")
                            .default_open(true)
                            .show(ui, |ui| {
                                self.draw_highlight_settings(ctx, ui);
                            });

                        CollapsingHeader::new("Export Settings")
                            .default_open(true)
                            .show(ui, |ui| {
                                self.draw_export_settings(ctx, ui);
                            });
                    });
                    ui.add_space(20.0);
                    ui.separator();
                    ui.collapsing("Debug logs:", |ui| {
                        egui_logger::logger_ui().show(ui);
                    });

                    match self.file_dialog_state {
                        FileDialogState::Open => {
                            if let Some(path) = self
                                .file_dialog
                                .update_with_right_panel_ui(ctx, &mut |ui, dia| {
                                    self.information_panel.ui(ui, dia);
                                })
                                .picked()
                            {
                                self.picked_path = path.to_path_buf();
                                self.file_opened = true;
                                self.file_dialog_state = FileDialogState::None;
                                if let Err(e) = self.load_tx.send(self.picked_path.clone()) {
                                    log::error!("load_tx thread send failed: {:?}", e);
                                }
                            }
                        }
                        FileDialogState::SavePlot => {
                            if let Some(path) = self.file_dialog.update(ctx).picked() {
                                self.picked_path = path.to_path_buf();
                                self.file_dialog_state = FileDialogState::None;
                                self.picked_path.set_extension("png");

                                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                                    Default::default(),
                                ));
                            }
                        }
                        FileDialogState::Save => {
                            if let Some(path) = self.file_dialog.update(ctx).picked() {
                                self.picked_path = path.to_path_buf();
                                self.file_dialog_state = FileDialogState::None;
                                self.picked_path.set_extension("csv");

                                if let Err(e) = self.save_tx.send(FileOptions {
                                    file_path: self.picked_path.clone(),
                                    save_absolute_time: self.gui_conf.save_absolute_time,
                                    save_raw_traffic: self.save_raw,
                                    names: self.labels.clone(),
                                }) {
                                    log::error!("save_tx thread send failed: {:?}", e);
                                }
                            }
                        }
                        FileDialogState::None => {}
                    }
                });
            });
    }

    fn paint_connection_indicator(&self, ui: &mut egui::Ui) {
        let (color, color_stroke) = if !self.connected_to_device {
            ui.add(egui::Spinner::new());
            (Color32::DARK_RED, Color32::RED)
        } else {
            (Color32::DARK_GREEN, Color32::GREEN)
        };

        let radius = ui.spacing().interact_size.y * 0.375;
        let center = egui::pos2(
            ui.next_widget_position().x + ui.spacing().interact_size.x * 0.5,
            ui.next_widget_position().y,
        );
        ui.painter()
            .circle(center, radius, color, egui::Stroke::new(1.0, color_stroke));
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Ok(read_guard) = self.connected_lock.read() {
            self.connected_to_device = *read_guard;
        }
        self.draw_central_panel(ctx);
        self.draw_side_panel(ctx, frame);

        self.gui_conf.x = ctx.used_size().x;
        self.gui_conf.y = ctx.used_size().y;

        // Check for returned screenshot:
        let screenshot = ctx.input(|i| {
            for event in &i.raw.events {
                if let egui::Event::Screenshot { image, .. } = event {
                    return Some(image.clone());
                }
            }
            None
        });

        if let (Some(screenshot), Some(plot_location)) = (screenshot, self.plot_location) {
            // for a full size application, we should put this in a different thread,
            // so that the GUI doesn't lag during saving

            let pixels_per_point = ctx.pixels_per_point();
            let plot = screenshot.region(&plot_location, Some(pixels_per_point));
            // save the plot to png
            image::save_buffer(
                &self.picked_path,
                plot.as_raw(),
                plot.width() as u32,
                plot.height() as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            log::info!("Image saved to {:?}.", self.picked_path);
        }
    }

    fn save(&mut self, _storage: &mut dyn Storage) {
        save_serial_settings(&self.serial_devices);
        if let Err(err) = self.gui_conf.save(&APP_INFO, PREFERENCES_KEY) {
            log::error!("gui settings save failed: {:?}", err);
        }
    }
}
