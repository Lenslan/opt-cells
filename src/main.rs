use std::io::Read;
use std::process::ExitCode;

use clap::Parser;
use opt_cells::cli::Args;
use opt_cells::{run_pipeline, RunInputs};

fn main() -> ExitCode {
    let args = Args::parse();
    let (text, name) = match read_input(&args.input) {
        Ok(p) => p,
        Err(e) => { eprintln!("error: {}", e); return ExitCode::from(1); }
    };
    let result = run_pipeline(RunInputs {
        input_path: name,
        input_text: text,
        library_path: args.library.clone(),
    });
    match result {
        Ok((report_text, _display)) => {
            let final_text = if args.quiet {
                let n = report_text
                    .lines()
                    .find(|l| l.contains("Total cells used"))
                    .and_then(|l| l.split(':').nth(1).map(|s| s.trim().to_string()))
                    .unwrap_or_default();
                format!("total cells: {}\n", n)
            } else {
                report_text
            };
            if let Some(out_path) = &args.output {
                std::fs::write(out_path, &final_text).expect("write");
            } else {
                print!("{}", final_text);
            }
            ExitCode::from(0)
        }
        Err(err) => {
            let _ = err.render(&mut std::io::stderr());
            ExitCode::from(1)
        }
    }
}

fn read_input(arg: &str) -> std::io::Result<(String, String)> {
    if arg == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok((buf, "<stdin>".to_string()))
    } else {
        let text = std::fs::read_to_string(arg)?;
        Ok((text, arg.to_string()))
    }
}
