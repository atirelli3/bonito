//! Finding a function by what the user typed.

use crate::kuna::Function;

/// The function `word` names: its name, an alias or its entry address (`0x…`); `contained` takes any address in it.
// ponytail: the first match in address order wins; duplicate names, and an alias equal to another's name, are not told apart.
pub(super) fn function_named<'a>(
    functions: &'a [Function],
    word: &str,
    contained: bool,
) -> Option<&'a Function> {
    let address = word
        .strip_prefix("0x")
        .or_else(|| word.strip_prefix("0X"))
        .and_then(|digits| u64::from_str_radix(digits, 16).ok());
    let entry = |function: &&Function| Some(function.address) == address;
    let inside = |function: &&Function| {
        let end = function.address.saturating_add(function.size.max(1));
        contained && address.is_some_and(|at| (function.address..end).contains(&at))
    };
    let named = |function: &&Function| {
        function.name.as_str() == word
            || function.aliases.iter().any(|alias| alias.as_str() == word)
    };
    functions
        .iter()
        .find(entry)
        .or_else(|| functions.iter().find(inside))
        .or_else(|| functions.iter().find(named))
}
