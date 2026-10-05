use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut frontend: Option<PathBuf> = None;
    let mut frozen: Option<PathBuf> = None;
    let mut cases: Option<PathBuf> = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--emit-frontend" => {
                frontend = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--append-frozen" => {
                frozen = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--emit-cases" => {
                cases = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    if let Some(path) = frontend {
        std::fs::write(&path, board_schema::frontend::Frontend::source()).unwrap();
        eprintln!("wrote {}", path.display());
    }
    if let Some(path) = cases {
        #[cfg(not(feature = "legacy"))]
        {
            std::fs::write(&path, board_schema::cases::Cases::fixture()).unwrap();
            eprintln!("wrote {}", path.display());
        }
        #[cfg(feature = "legacy")]
        {
            let _ = path;
            eprintln!("the case fixture requires the default build");
            std::process::exit(2);
        }
    }
    if let Some(dir) = frozen {
        #[cfg(not(feature = "legacy"))]
        match board_schema::corpus::Corpus::append(&dir) {
            Ok(written) => {
                for relative in &written {
                    eprintln!("appended {relative}");
                }
                eprintln!("appended {} entries to {}", written.len(), dir.display());
            }
            Err(problems) => {
                for problem in &problems {
                    eprintln!("{problem}");
                }
                std::process::exit(2);
            }
        }
        #[cfg(feature = "legacy")]
        {
            let _ = dir;
            eprintln!("the frozen corpus requires the default build");
            std::process::exit(2);
        }
    }
}
