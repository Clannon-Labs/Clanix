
// Tests are not yet properly thought of, so they 
// dont exist here yet, after clanix becomes capable of 
// doing even a very small thing meaningfully, many tests
// will be added.. that's for sure!

use std::process::Command;

fn main() {
    let output = Command::new("echo")
        .arg("hey")
        .output()
        .unwrap();

    println!("{:?}", output)
}
