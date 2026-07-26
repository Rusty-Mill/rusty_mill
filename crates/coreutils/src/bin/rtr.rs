//! `rtr` — translate or delete characters

use std::collections::HashMap;
use std::env;
use std::io::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let delete = args.contains(&"-d".to_string());
    let non_opts: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();

    if non_opts.is_empty() {
        eprintln!("Usage: rtr [-d] SET1 [SET2]");
        std::process::exit(1);
    }

    let set1 = non_opts[0];
    let set2 = if non_opts.len() > 1 { non_opts[1] } else { "" };

    let mut map = HashMap::new();
    let set1_chars: Vec<char> = set1.chars().collect();
    let set2_chars: Vec<char> = set2.chars().collect();

    if !delete && !set2_chars.is_empty() {
        for (idx, &ch1) in set1_chars.iter().enumerate() {
            let ch2 = if idx < set2_chars.len() {
                set2_chars[idx]
            } else {
                *set2_chars.last().unwrap()
            };
            map.insert(ch1, ch2);
        }
    }

    let mut buffer = Vec::new();
    io::stdin().read_to_end(&mut buffer)?;

    let input = String::from_utf8_lossy(&buffer);
    let mut output = String::new();

    for ch in input.chars() {
        if delete {
            if !set1_chars.contains(&ch) {
                output.push(ch);
            }
        } else if let Some(&replacement) = map.get(&ch) {
            output.push(replacement);
        } else {
            output.push(ch);
        }
    }

    print!("{}", output);
    Ok(())
}
