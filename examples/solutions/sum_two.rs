use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut values = input.split_whitespace().map(|value| value.parse::<i64>().unwrap());
    let left = values.next().unwrap();
    let right = values.next().unwrap();
    println!("{}", left + right);
}
