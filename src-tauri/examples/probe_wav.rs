use lofty::prelude::*;
use lofty::probe::Probe;
use std::fs;
use std::io::Cursor;

fn dump(label: &str, bytes: &[u8]) {
    let mut reader = Cursor::new(bytes);
    match Probe::new(&mut reader)
        .guess_file_type()
        .map_err(|e| e.to_string())
        .and_then(|p| p.read().map_err(|e| e.to_string()))
    {
        Ok(tagged) => {
            println!(
                "[{label}] ok file_type={:?} tags={:?} duration={:?}",
                tagged.file_type(),
                tagged.tags().iter()
                    .map(|t| (t.tag_type(), t.items().count()))
                    .collect::<Vec<_>>(),
                tagged.properties().duration(),
            );
            for tag in tagged.tags() {
                for item in tag.items() {
                    println!("    {:?} = {:?}", item.key(), item.value());
                }
            }
        }
        Err(e) => println!("[{label}] error: {e}"),
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe_wav <file.wav>");
    let all = fs::read(&path).unwrap();
    println!("file size = {}", all.len());

    dump("whole file", &all);

    // 对照真实的 Range 分级：HEAD_RANGES = [1.5 MiB, 6 MiB]
    // (src-tauri/src/commands/media.rs)
    let sizes: &[usize] = &[1_572_864, 6_291_456];
    for &size in sizes {
        let head = &all[..all.len().min(size)];
        dump(&format!("head {size} bytes"), head);
    }
}
