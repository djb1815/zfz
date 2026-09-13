fn main() {
    if let Err(error) = zfz::cli::run_from_env() {
        eprintln!("zfz: {error}");
        std::process::exit(error.exit_code().into());
    }
}
