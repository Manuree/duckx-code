// Find every use of a name.
//
// Put the cursor on a name and press Shift+F12.
// F12 jumps to the place that name is defined.
//
// greet is used once, in main.
// farewell is used in main and in greet.
// score is used twice on one line.
//
// In the list, Up and Down move. Enter or Tab jumps. Esc closes.
// Ctrl+O goes back. Ctrl+Shift+O goes forward again.
// Shift+F2 asks for a new name and changes each use.

fn main() {
    greet("Ada");
    farewell("Ada");
    let total = score(2) + score(3);
    println!("{total}");
}

fn greet(name: &str) {
    println!("Hello, {name}");
    farewell(name);
}

fn farewell(name: &str) {
    println!("Goodbye, {name}");
}

fn score(value: i32) -> i32 {
    value * 10
}
