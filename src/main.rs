#![warn(clippy::unwrap_used)]

use std::{io::Cursor, path::PathBuf, sync::mpsc::channel};

use anyhow::anyhow;
use clap::Parser;
use image::{
    ImageFormat, ImageReader,
    imageops::{FilterType, replace},
};
use inquire::{Select, prompt_u32};
use notify::{Event, EventKind, RecursiveMode, Watcher, event::CreateKind, recommended_watcher};
use printers::{
    common::{base::job::PrinterJobOptions, converters::Converter},
    get_printers,
};
use tracing::{debug, error, info, info_span, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber, fmt::format::FmtSpan, util::SubscriberInitExt};

#[derive(Parser, Debug)]
struct Args {
    /// Directory to watch
    r#in: PathBuf,
    /// Background image
    back: PathBuf,
    /// Top of region to composite into
    top: i64,
    /// Left of region to composite into
    left: i64,
    /// Width of region to composite into
    width: u32,
    /// Height of region to composite into
    height: u32,
}

fn main() -> anyhow::Result<()> {
    FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::ENTER | FmtSpan::CLOSE)
        .finish()
        .init();
    debug!("Parsing args");
    let args = Args::parse();
    debug!("Parsed args\n{args:#?}");
    let printers = get_printers();
    debug!("Prompting for printer in\n{printers:#?}");
    let names = printers
        .iter()
        .map(|printer| printer.name.clone())
        .collect();
    let printer = printers
        .into_iter()
        .nth(Select::new("Select a printer", names).raw_prompt()?.index)
        .ok_or_else(|| anyhow!("No printers or invalid selection"))?;
    debug!("Selected printer\n{printer:#?}");

    debug!("Opening background image at {}", args.back.display());
    let bottom = ImageReader::open(args.back)?.decode()?;
    let (x, y) = (args.left, args.top);

    let (tx, rx) = channel();

    let mut watcher = recommended_watcher(move |event: notify::Result<Event>| {
        debug!("Event received: {event:#?}");
        if let Ok(event) = event
            && event.kind == EventKind::Create(CreateKind::File)
        {
            match event.paths.into_iter().next() {
                Some(path) => {
                    if tx.send(path).is_err() {
                        warn!("Receiver was dropped, ignoring event");
                    }
                }
                None => {
                    error!("No paths in event");
                }
            }
        }
    })?;
    watcher.watch(&args.r#in, RecursiveMode::NonRecursive)?;
    info!("Ready");
    for path in &rx {
        info_span!("process event", path = %path.display()).in_scope(
            || -> anyhow::Result<()> {
                info!("Event received");
                let mut bottom = bottom.clone();
                info!("Compositing");
                let image = ImageReader::open(path)?
                    .with_guessed_format()?
                    .decode()?
                    .resize_to_fill(args.width, args.height, FilterType::Gaussian)
                    .grayscale();
                replace(&mut bottom, &image, x, y);
                let mut bytes = Vec::new();
                bottom.write_to(Cursor::new(&mut bytes), ImageFormat::Png)?;
                info!("Printing");
                let copies = prompt_u32("How many copies?")?.to_string();
                printer
                    .print(
                        &bytes,
                        PrinterJobOptions {
                            name: None,
                            raw_properties: &[
                                ("copies", &copies),
                                ("print-color-mode", "monochrome"),
                                ("ColorModel", "Mono"),
                                ("EPIJ_Ink", "0"),
                            ],
                            converter: Converter::None,
                        },
                    )
                    .map_err(|error| anyhow!("Print error: {error:#?}"))?;
                Ok(())
            },
        )?;
    }
    Ok(())
}
