use std::{mem, thread};

use regex::Regex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tap::Tap;

#[derive(Serialize)]
struct RB {
    action: String,
    version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

fn anki_request<T: DeserializeOwned>(
    action: String,
    params: Option<serde_json::Value>,
) -> anyhow::Result<T> {
    let url = format!("http://localhost:8765");
    let body = RB {
        action,
        version: 6,
        params,
    };

    let data: T = ureq::post(&url).send_json(body)?.into_json()?;

    Ok(data)
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
struct Field {
    value: String,
    order: i64,
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
struct GuiCurrentCard {
    error: Option<serde_json::Value>,
    result: Option<GuiCurrentCardResult>,
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuiCurrentCardResult {
    deck_name: String,
    fields: GuiCurrentCardFields,
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GuiCurrentCardFields {
    kanji: Field,
    kana: Field,
    sentence_front: Field,
    sentence_back: Field,
    picture: Field,
    kanken_audio: Field,
    kanken_level: Field,
    meaning: Field,
    diagram: Field,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
struct GuiAddCardsFields {
    front: String,
    back: String,
    #[serde(rename = "Back Paragraph")]
    back_paragraph: String,
    audio_guide: String,
    audio: String,
}

fn setup_fonts(ctx: &egui::Context) {
    const NOTO_JP: &str = "noto-jp";
    const NOTO_TH: &str = "noto-th";

    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        NOTO_JP.into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/NotoSansJP/NotoSansJP-VariableFont_wght.ttf"
        ))
        .into(),
    );

    fonts.font_data.insert(
        NOTO_TH.into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/NotoSansThai/NotoSansThai-VariableFont_wdth,wght.ttf"
        ))
        .into(),
    );

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .tap_mut(|v| {
            let fs = &[NOTO_JP.into(), NOTO_TH.into()];
            v.extend_from_slice(fs);
            v.rotate_right(fs.len());
        });

    ctx.set_fonts(fonts);
}

#[derive(Debug)]
pub struct AppState {
    r: AppStateResistReset,
    front: String,
    audio_guide: String,
    follow_front: bool,
    back: String,
}

#[derive(Debug)]
pub struct AppStateResistReset {
    req_complete: crossbeam::channel::Receiver<GuiAddCardsFields>,
    req_complete_s: crossbeam::channel::Sender<GuiAddCardsFields>,
    fired: i64,
    prev_card: Option<GuiAddCardsFields>,
    maintain_prev: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            r: Default::default(),
            front: String::new(),
            audio_guide: String::new(),
            follow_front: true,
            back: String::new(),
        }
    }
}

impl Default for AppStateResistReset {
    fn default() -> Self {
        let (req_complete_s, req_complete) = crossbeam::channel::unbounded();
        Self {
            req_complete,
            req_complete_s,
            fired: 0,
            prev_card: None,
            maintain_prev: false,
        }
    }
}

impl AppState {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);

        Default::default()
    }

    fn reset(&mut self) -> &mut Self {
        *self = Self {
            r: mem::take(&mut self.r),
            ..Self::default()
        };
        self
    }

    fn audio_guide_follow(&mut self) -> &mut Self {
        self.audio_guide = create_audio_guide(&self.front);
        self
    }

    fn fire(&self, c: egui::Context) {
        let front = self.front.trim().to_owned();
        let audio_guide = self.audio_guide.trim().to_owned();
        let back = self.back.trim().replace('\n', "<br />");

        let prev_card = self.r.prev_card.clone();
        let sender = self.r.req_complete_s.clone();
        _ = thread::spawn(move || {
            let new_card = if let Some(mut p) = prev_card {
                if !front.is_empty() {
                    p.front = front;
                }
                if !back.is_empty() {
                    p.back = back;
                }
                if !audio_guide.is_empty() {
                    p.audio_guide = audio_guide;
                }
                p
            } else {
                let Ok(ccard) = anki_request::<GuiCurrentCard>("guiCurrentCard".into(), None)
                else {
                    return;
                };
                let Some(data) = ccard.result else {
                    return;
                };
                let fields = data.fields;
                let sentence = ammonia::Builder::empty().clean(&fields.sentence_back.value);

                let front = if front.is_empty() {
                    format!("{}[{}]", fields.kanji.value, fields.kana.value)
                } else {
                    front
                };
                let back = if back.is_empty() {
                    fields.meaning.value
                } else {
                    back
                };
                let back_paragraph = format!("{}\n{}", sentence, fields.picture.value)
                    .trim()
                    .replace('\n', "<br />");
                let audio_guide = if audio_guide.is_empty() {
                    fields.kanji.value
                } else {
                    audio_guide
                };
                let audio = fields.kanken_audio.value;

                GuiAddCardsFields {
                    front,
                    back,
                    back_paragraph,
                    audio_guide,
                    audio,
                }
            };

            let Ok(_) = anki_request::<serde_json::Value>(
                "guiAddCards".into(),
                Some(serde_json::json!({
                    "note": {
                        "deckName": "Immersion",
                        "modelName": "Immersion",
                        "fields": new_card,
                        "tags": [
                            "Immersion",
                            "from::KanKenDeck",
                        ],
                    },
                })),
            ) else {
                return;
            };

            _ = sender.send(new_card);
            c.request_repaint();
        });
    }
}

fn create_audio_guide(s: &str) -> String {
    let s = s.replace(&['(', ')', '{', '}', ' '], "");
    let r = Regex::new(r"\[[^\]]*\]").unwrap();
    r.replace_all(&s, "").to_string()
}
#[cfg(test)]
mod tests {
    use super::create_audio_guide;
    #[test]
    fn test_create_audio_guide() {
        assert_eq!(
            create_audio_guide("ab[]c[de]f gh[ij]k (lmn) {op}"),
            "abcfghklmnop".to_string()
        );
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame {
                inner_margin: egui::Margin::same(10.),
                ..Default::default()
            })
            .show(ctx, |ui| {
                if let Ok(new_card) = self.r.req_complete.try_recv() {
                    self.reset();
                    self.r.prev_card = if self.r.maintain_prev {
                        Some(new_card)
                    } else {
                        None
                    };
                }

                ui.heading("Anki Copy Card");

                egui::Grid::new("main-grid")
                    .spacing([4.0, 4.0])
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Custom Front:");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.front)
                                    .hint_text("噛[か]み 殺[ころ]す"),
                            )
                            .changed()
                            && self.follow_front
                        {
                            self.audio_guide_follow();
                        }
                        ui.end_row();

                        ui.label("Custom Audio Guide:");
                        ui.vertical(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.audio_guide)
                                    .hint_text("噛み殺す"),
                            );
                            if ui
                                .checkbox(&mut self.follow_front, "Follow Front")
                                .changed()
                                && self.follow_front
                            {
                                self.audio_guide_follow();
                            };
                        });
                        ui.end_row();

                        ui.label("Back:");
                        ui.add(
                            egui::TextEdit::multiline(&mut self.back)
                                .hint_text("to stifle a smile, yawn, etc.\nto bite to death"),
                        );
                        ui.end_row();
                    });

                ui.vertical(|ui| {
                    ui.checkbox(
                        &mut self.r.maintain_prev,
                        "Maintain current card for next round",
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Fire").clicked() {
                            let fired = self.r.fired;
                            self.fire(ctx.clone());
                            self.r.fired = fired + 1;
                        }
                        if ui.button("Reset").clicked() {
                            self.reset();
                        }
                    });

                    if let Some(p) = &self.r.prev_card {
                        ui.label(format!(
                            "Firing will be based on previous card fired: {}",
                            p.front
                        ));
                        if ui.button("Reset Previous Card").clicked() {
                            self.r.prev_card = None;
                        }
                    }

                    ui.label(format!("Fired: {}", self.r.fired));
                });
            });
    }
}

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800., 600.]),
        ..Default::default()
    };

    eframe::run_native(
        "anki-copy-card-egui",
        native_options,
        Box::new(|cc| Ok(Box::new(AppState::new(cc)))),
    )
}
