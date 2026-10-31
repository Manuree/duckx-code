// Put the cursor on a bracket and press Ctrl+].
// It jumps to the pair: ( ) [ ] or { }.
// Ctrl+Shift+] selects through to that bracket.

fn main() {
    let items = [1, 2, 3];
    if items.len() > 0 {
        println!("{}", items[0]);
    }
}
