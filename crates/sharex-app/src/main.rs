fn main() {
    if let Err(message) = sharex_app::run() {
        eprintln!("sharex: {message}");
        std::process::exit(1);
    }
}
