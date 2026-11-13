//! A separate root window: eframe child viewports do not present on our GPU.
use crate::config::Place;
use eframe::egui;
use std::sync::mpsc::{self, Receiver};

#[derive(Default)]
pub struct PlaceSearch {
    query: String,
    results: Vec<Place>,
    pending: Option<Receiver<Result<Vec<Place>, String>>>,
    status: String,
    focused: bool,
}

impl PlaceSearch {
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Place> {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok(results) => {
                            self.status = if results.is_empty() {
                                "No places found. Try another name.".into()
                            } else {
                                "Choose a place:".into()
                            };
                            self.results = results;
                        }
                        Err(error) => self.status = error,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.status = "Search stopped. Try again.".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        ui.label("Place name");
        ui.hyperlink_to(
            "Place search by Open-Meteo / GeoNames",
            "https://open-meteo.com/en/docs/geocoding-api",
        );
        let input = ui.text_edit_singleline(&mut self.query);
        if !self.focused {
            input.request_focus();
            self.focused = true;
        }
        let enabled = self.pending.is_none() && !self.query.trim().is_empty();
        let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui
            .add_enabled(enabled, egui::Button::new("Search"))
            .clicked()
            || enter)
            && enabled
        {
            let (tx, rx) = mpsc::channel();
            let query = self.query.trim().to_owned();
            let ctx = ui.ctx().clone();
            self.pending = Some(rx);
            self.results.clear();
            self.status = "Searching…".into();
            std::thread::spawn(move || {
                let _ = tx.send(crate::widgets::weather::search_places(&query));
                ctx.request_repaint();
            });
        }
        if self.pending.is_some() {
            ui.spinner();
        }
        ui.label(&self.status);
        let mut selected = None;
        if !self.results.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("place-results")
                .max_height(220.0)
                .show(ui, |ui| {
                    for place in &self.results {
                        if ui.button(&place.name).clicked() {
                            selected = Some(place.clone());
                        }
                    }
                });
        }
        selected
    }
}
