mod cli;
mod preferences;
mod parser;
mod layout;
mod backend;

fn main() {
    let _args = cli::parse();
    println!("genechart");
}
