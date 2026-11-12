#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let phone = args.iter().any(|a| a == "--phone");
    let files = args.into_iter().filter(|a| !a.starts_with("--")).collect();
    velour::run_desktop(files, phone)
}
