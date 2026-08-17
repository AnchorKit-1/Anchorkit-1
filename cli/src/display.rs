use stellar_xdr::curr::{AccountId, ContractId, Hash, PublicKey, ScAddress, ScVal, Uint256};

/// Renders a decoded contract return value as a readable string. Handles the
/// shapes this playground's supported methods actually return -- bool, u64,
/// addresses, symbols, and the map/vec encoding `#[contracttype]` structs and
/// enums use -- generically, so it doesn't need to know `Attestation`'s
/// exact field layout to display it. Anything else falls back to `Debug`
/// rather than failing to print.
pub fn format_scval(val: &ScVal) -> String {
    match val {
        ScVal::Bool(b) => b.to_string(),
        ScVal::Void => "()".to_string(),
        ScVal::U32(n) => n.to_string(),
        ScVal::I32(n) => n.to_string(),
        ScVal::U64(n) => n.to_string(),
        ScVal::I64(n) => n.to_string(),
        ScVal::Bytes(bytes) => format!("0x{}", hex_encode(bytes.0.iter().copied())),
        ScVal::String(s) => s.0.to_string(),
        ScVal::Symbol(s) => s.0.to_string(),
        ScVal::Address(addr) => format_address(addr),
        ScVal::Vec(Some(vec)) => {
            let items: Vec<String> = vec.0.iter().map(format_scval).collect();
            format!("[{}]", items.join(", "))
        }
        ScVal::Vec(None) => "[]".to_string(),
        ScVal::Map(Some(map)) => {
            let items: Vec<String> = map
                .0
                .iter()
                .map(|entry| format!("{}: {}", format_scval(&entry.key), format_scval(&entry.val)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        ScVal::Map(None) => "{}".to_string(),
        other => format!("{other:?}"),
    }
}

fn format_address(addr: &ScAddress) -> String {
    match addr {
        ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(bytes)))) => {
            // `stellar_strkey`'s own inherent `to_string()` returns a
            // stack-allocated `heapless::String`, not `std::string::String`
            // (it's usable in no_std contexts) -- format! goes through
            // `Display` instead, which always yields `std::string::String`.
            format!("{}", stellar_strkey::ed25519::PublicKey(*bytes))
        }
        ScAddress::Contract(ContractId(Hash(bytes))) => {
            format!("{}", stellar_strkey::Contract(*bytes))
        }
        other => format!("{other:?}"),
    }
}

fn hex_encode(bytes: impl IntoIterator<Item = u8>) -> String {
    bytes.into_iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{ScBytes, ScMap, ScMapEntry, ScString, ScSymbol, ScVec};

    #[test]
    fn formats_scalars() {
        assert_eq!(format_scval(&ScVal::Bool(true)), "true");
        assert_eq!(format_scval(&ScVal::U64(42)), "42");
        assert_eq!(format_scval(&ScVal::Void), "()");
    }

    #[test]
    fn formats_string_and_symbol() {
        let s = ScVal::String(ScString("hello".parse().unwrap()));
        assert_eq!(format_scval(&s), "hello");
        let sym = ScVal::Symbol(ScSymbol("active".try_into().unwrap()));
        assert_eq!(format_scval(&sym), "active");
    }

    #[test]
    fn formats_bytes_as_hex() {
        let bytes = ScVal::Bytes(ScBytes(vec![0xde, 0xad, 0xbe, 0xef].try_into().unwrap()));
        assert_eq!(format_scval(&bytes), "0xdeadbeef");
    }

    #[test]
    fn formats_nested_vec_and_map() {
        let inner_vec = ScVal::Vec(Some(ScVec(vec![ScVal::U32(1), ScVal::U32(2)].try_into().unwrap())));
        assert_eq!(format_scval(&inner_vec), "[1, 2]");

        let map = ScVal::Map(Some(ScMap(
            vec![ScMapEntry {
                key: ScVal::Symbol(ScSymbol("status".try_into().unwrap())),
                val: ScVal::Bool(true),
            }]
            .try_into()
            .unwrap(),
        )));
        assert_eq!(format_scval(&map), "{status: true}");
    }
}
