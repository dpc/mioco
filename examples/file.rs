extern crate mioco;

use mioco::fs;
use std::io::BufRead;

fn main() {

    mioco::spawn(|| -> std::io::Result<()> {
        let file = fs::File::open("/etc/passwd")?;
        let buffile = std::io::BufReader::new(file);

        for line in buffile.lines() {
            println!("{}", line.unwrap());
        }
        Ok(())
    }).join().unwrap().unwrap();
}
