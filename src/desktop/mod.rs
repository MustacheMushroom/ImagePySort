//! Native desktop application shell for MediaSift.

mod app;
mod operations;
mod state;
mod ui;

/// Launch the native MediaSift desktop application.
pub fn run() -> eframe::Result<()> {
    app::run()
}
