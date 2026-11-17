fn main() -> std::process::ExitCode {
    pansy_cli::run(std::env::args().skip(1).collect())
}
