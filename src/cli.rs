use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "opt-cells",
    version,
    about = "Map combinational logic to minimum-cell-count cell library implementation"
)]
pub struct Args {
    /// Path to DSL input file, or "-" for stdin.
    pub input: String,

    /// Cell library TOML file.
    #[arg(short = 'l', long = "library", default_value="./lib/basic.toml")]
    pub library: String,

    /// Write report to file (default: stdout).
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,

    /// Only show cell-count summary.
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,
}
