#![allow(unused_imports)]

use egui::Image;
use poll_promise::Promise;
use serde::{Deserialize, Serialize};
use turbosql::{execute, select, update, Turbosql};

#[derive(Turbosql, Default)]
struct Card {
	rowid: Option<i64>,
	#[turbosql(sql_default = false)]
	deleted: bool,
	#[turbosql(sql_default = "(no title)")]
	title: String,
	#[turbosql(sql_default = "question goes here")]
	question: String,
	#[turbosql(sql_default = "answer goes here")]
	answer: String,
	#[turbosql(sql_default = 0)]
	last_question_viewed_ms: i64,
	#[turbosql(sql_default = 0)]
	last_answer_viewed_ms: i64,
}

#[allow(clippy::enum_variant_names)]
#[derive(Serialize, Deserialize, Default)]
enum Action {
	#[default]
	NoAction,
	ViewedQuestion,
	ViewedAnswer,
	Responded {
		correct: bool,
	},
}

#[derive(Turbosql, Default)]
struct CardLog {
	rowid: Option<i64>,
	#[turbosql(sql_default = 0)]
	card_id: i64,
	#[turbosql(sql_default = 0)]
	time_ms: i64,
	#[turbosql(sql_default = "")]
	action: Action,
}

struct Resource {
	/// HTTP response
	response: ehttp::Response,

	text: Option<String>,

	/// If set, the response was an image.
	image: Option<Image<'static>>,

	/// If set, the response was text with some supported syntax highlighting (e.g. ".rs" or ".md").
	colored_text: Option<ColoredText>,
}

impl Resource {
	fn from_response(ctx: &egui::Context, response: ehttp::Response) -> Self {
		let content_type = response.content_type().unwrap_or_default();
		if content_type.starts_with("image/") {
			ctx.include_bytes(response.url.clone(), response.bytes.clone());
			let image = Image::from_uri(response.url.clone());

			Self { response, text: None, colored_text: None, image: Some(image) }
		} else {
			let text = response.text();
			let colored_text = text.and_then(|text| syntax_highlighting(ctx, &response, text));
			let text = text.map(|text| text.to_owned());

			Self { response, text, colored_text, image: None }
		}
	}
}

#[derive(Default, Deserialize, Serialize)]
pub struct HttpApp {
	url: String,
	line_selected: i64,
	title_text: String,
	question_text: String,
	answer_text: String,

	#[serde(skip)]
	promise: Option<Promise<ehttp::Result<Resource>>>,
}

impl HttpApp {
	pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
		cc.egui_ctx.style_mut(|s| s.visuals.override_text_color = Some(egui::Color32::WHITE));

		egui_extras::install_image_loaders(&cc.egui_ctx);

		// Load previous app state (if any).
		// Note that you must enable the `persistence` feature for this to work.
		// if let Some(storage) = cc.storage {
		//     return eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
		// }

		Self::default()
	}
}

impl eframe::App for HttpApp {
	fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
		ctx.input(|i| {
			if i.key_pressed(egui::Key::ArrowDown) {
				self.line_selected =
					select!(i64 "MIN(rowid) FROM card WHERE NOT deleted AND rowid > " self.line_selected)
						.unwrap_or(self.line_selected);
			} else if i.key_pressed(egui::Key::ArrowUp) {
				self.line_selected =
					select!(i64 "MAX(rowid) FROM card WHERE NOT deleted AND rowid < " self.line_selected)
						.unwrap_or(self.line_selected);
			} else if i.key_pressed(egui::Key::Enter) {
				Card::default().insert().unwrap();
			} else if i.key_pressed(egui::Key::Backspace) {
				let _ = update!("card SET deleted = 1 WHERE rowid = " self.line_selected);
				self.line_selected =
					select!(i64 "MIN(rowid) FROM card WHERE NOT deleted AND rowid > " self.line_selected)
						.unwrap_or(0);
			}
		});

		let cards = select!(Vec<Card> "WHERE NOT deleted").unwrap();

		egui::SidePanel::left("left_panel").show(ctx, |ui| {
			egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
				for card in cards {
					let i = card.rowid.unwrap();
					if ui.selectable_label(i == self.line_selected, format!("{}: {}", i, card.title)).clicked() {
						self.line_selected = i;
					}
				}
			});
		});

		let card = select!(Card "WHERE rowid = " self.line_selected).unwrap_or_default();

		egui::CentralPanel::default().show(ctx, |ui| {
			let prev_url = self.url.clone();
			let trigger_fetch = ui_url(ui, frame, &mut self.url);

			if trigger_fetch {
				let ctx = ctx.clone();
				let (sender, promise) = Promise::new();
				let request = ehttp::Request::get(&self.url);
				ehttp::fetch(request, move |response| {
					ctx.forget_image(&prev_url);
					ctx.request_repaint(); // wake up UI thread
					let resource = response.map(|response| Resource::from_response(&ctx, response));
					sender.send(resource);
				});
				self.promise = Some(promise);
			}

			ui.label(format!("Selected line: {}", self.line_selected));
			ui.label(format!("title: {}", card.title));
			ui.label(format!("question: {}", card.question));
			ui.label(format!("answer: {}", card.answer));

			let mut title_text = card.title.clone();
			ui.label("title:");
			if ui.text_edit_multiline(&mut title_text).changed() {
				let _ = update!("card SET title = " title_text " WHERE rowid = " self.line_selected);
			}

			let mut question_text = card.question.clone();
			ui.label("question:");
			if ui.text_edit_multiline(&mut question_text).changed() {
				let _ = update!("card SET question = " question_text " WHERE rowid = " self.line_selected);
			}

			let mut answer_text = card.answer.clone();
			ui.label("answer:");
			if ui.text_edit_multiline(&mut answer_text).changed() {
				let _ = update!("card SET answer = " answer_text " WHERE rowid = " self.line_selected);
			}

			ui.separator();

			if let Some(promise) = &self.promise {
				if let Some(result) = promise.ready() {
					match result {
						Ok(resource) => {
							ui_resource(ui, resource);
						}
						Err(error) => {
							// This should only happen if the fetch API isn't available or something similar.
							ui
								.colored_label(ui.visuals().error_fg_color, if error.is_empty() { "Error" } else { error });
						}
					}
				} else {
					ui.spinner();
				}
			}
		});
	}
}

fn ui_url(ui: &mut egui::Ui, _frame: &mut eframe::Frame, url: &mut String) -> bool {
	let mut trigger_fetch = false;

	ui.horizontal(|ui| {
		ui.label("URL:");
		trigger_fetch |=
			ui.add(egui::TextEdit::singleline(url).desired_width(f32::INFINITY)).lost_focus();
	});

	ui.horizontal(|ui| {
		if ui.button("Random image").clicked() {
			let seed = ui.input(|i| i.time);
			let side = 640;
			*url = format!("https://picsum.photos/seed/{seed}/{side}");
			trigger_fetch = true;
		}
	});

	trigger_fetch
}

fn ui_resource(ui: &mut egui::Ui, resource: &Resource) {
	let Resource { response, text, image, colored_text } = resource;

	ui.monospace(format!("url:          {}", response.url));
	ui.monospace(format!("status:       {} ({})", response.status, response.status_text));
	ui.monospace(format!("content-type: {}", response.content_type().unwrap_or_default()));
	ui.monospace(format!("size:         {:.1} kB", response.bytes.len() as f32 / 1000.0));

	ui.separator();

	egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
		egui::CollapsingHeader::new("Response headers").default_open(false).show(ui, |ui| {
			egui::Grid::new("response_headers")
				.spacing(egui::vec2(ui.spacing().item_spacing.x * 2.0, 0.0))
				.show(ui, |ui| {
					for header in &response.headers {
						ui.label(header.0);
						ui.label(header.1);
						ui.end_row();
					}
				})
		});

		ui.separator();

		if let Some(text) = &text {
			let tooltip = "Click to copy the response body";
			if ui.button("📋").on_hover_text(tooltip).clicked() {
				ui.ctx().copy_text(text.clone());
			}
			ui.separator();
		}

		if let Some(image) = image {
			ui.add(image.clone());
		} else if let Some(colored_text) = colored_text {
			colored_text.ui(ui);
		} else if let Some(text) = &text {
			selectable_text(ui, text);
		} else {
			ui.monospace("[binary]");
		}
	});
}

fn selectable_text(ui: &mut egui::Ui, mut text: &str) {
	ui.add(
		egui::TextEdit::multiline(&mut text)
			.desired_width(f32::INFINITY)
			.font(egui::TextStyle::Monospace),
	);
}

// ----------------------------------------------------------------------------
// Syntax highlighting:

fn syntax_highlighting(
	ctx: &egui::Context,
	response: &ehttp::Response,
	text: &str,
) -> Option<ColoredText> {
	let extension_and_rest: Vec<&str> = response.url.rsplitn(2, '.').collect();
	let extension = extension_and_rest.first()?;
	let theme = egui_extras::syntax_highlighting::CodeTheme::from_style(&ctx.style());
	Some(ColoredText(egui_extras::syntax_highlighting::highlight(ctx, &theme, text, extension)))
}

struct ColoredText(egui::text::LayoutJob);

impl ColoredText {
	pub fn ui(&self, ui: &mut egui::Ui) {
		if true {
			// Selectable text:
			let mut layouter = |ui: &egui::Ui, _string: &str, wrap_width: f32| {
				let mut layout_job = self.0.clone();
				layout_job.wrap.max_width = wrap_width;
				ui.fonts(|f| f.layout_job(layout_job))
			};

			let mut text = self.0.text.as_str();
			ui.add(
				egui::TextEdit::multiline(&mut text)
					.font(egui::TextStyle::Monospace)
					.desired_width(f32::INFINITY)
					.layouter(&mut layouter),
			);
		} else {
			let mut job = self.0.clone();
			job.wrap.max_width = ui.available_width();
			let galley = ui.fonts(|f| f.layout_job(job));
			let (response, painter) = ui.allocate_painter(galley.size(), egui::Sense::hover());
			painter.add(egui::Shape::galley(response.rect.min, galley, ui.visuals().text_color()));
		}
	}
}
