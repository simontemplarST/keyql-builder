#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod query;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let example = std::env::args().any(|a| a == "--example");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("eDiscovery Search Request")
            .with_app_id("keyql-builder")
            .with_inner_size([1200.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "eDiscovery Search Request",
        options,
        Box::new(|cc| Ok(Box::new(app::KeyqlApp::new(cc, example)))),
    )
}

/// Browser build, served by `trunk serve`. Open the page with `?example` to
/// start on the review page with a sample request.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    let window = web_sys::window().expect("no window");
    let example = window
        .location()
        .search()
        .is_ok_and(|q| q.contains("example"));
    let canvas = window
        .document()
        .and_then(|d| d.get_element_by_id("app"))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("no <canvas id=\"app\">");

    wasm_bindgen_futures::spawn_local(async move {
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| Ok(Box::new(app::KeyqlApp::new(cc, example)))),
            )
            .await;
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        match started {
            // Fade the loading screen out (index.html), then drop it.
            Ok(()) => {
                if let Some(el) = doc.get_element_by_id("loading") {
                    el.set_class_name("done");
                    let remove = wasm_bindgen::closure::Closure::once_into_js(move || el.remove());
                    let _ = web_sys::window()
                        .expect("no window")
                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                            remove.unchecked_ref(),
                            400,
                        );
                }
            }
            Err(e) => {
                for (id, display) in [("bar", "none"), ("status", "none"), ("failed", "block")] {
                    if let Some(el) = doc.get_element_by_id(id) {
                        let _ = el.set_attribute("style", &format!("display:{display}"));
                    }
                }
                if let Some(el) = doc.get_element_by_id("failed-why") {
                    el.set_text_content(Some(
                        "Your browser may have graphics (WebGL) turned off. Try another browser.",
                    ));
                }
                web_sys::console::error_1(&e);
            }
        }
    });
}
