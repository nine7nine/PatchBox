//! Open Sound Control 1.0 messages, the subset NSM uses: int32, float32, string, int64, double,
//! blob, true/false/nil; bundles are unpacked into their messages.

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    I(i32),
    F(f32),
    S(String),
    H(i64),
    D(f64),
    B(Vec<u8>),
    T,
    False,
    Nil,
}

impl Arg {
    pub fn str(&self) -> Option<&str> {
        match self {
            Arg::S(s) => Some(s),
            _ => None,
        }
    }
    pub fn int(&self) -> Option<i64> {
        match self {
            Arg::I(i) => Some(*i as i64),
            Arg::H(h) => Some(*h),
            _ => None,
        }
    }
    pub fn float(&self) -> Option<f64> {
        match self {
            Arg::F(f) => Some(*f as f64),
            Arg::D(d) => Some(*d),
            Arg::I(i) => Some(*i as f64),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub path: String,
    pub args: Vec<Arg>,
}

impl Msg {
    pub fn new(path: &str, args: Vec<Arg>) -> Msg {
        Msg { path: path.to_string(), args }
    }
    pub fn s(&self, i: usize) -> Option<&str> {
        self.args.get(i).and_then(Arg::str)
    }
    pub fn i(&self, i: usize) -> Option<i64> {
        self.args.get(i).and_then(Arg::int)
    }
    pub fn f(&self, i: usize) -> Option<f64> {
        self.args.get(i).and_then(Arg::float)
    }
}

fn pad(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

fn put_str(v: &mut Vec<u8>, s: &str) {
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    pad(v);
}

pub fn encode(m: &Msg) -> Vec<u8> {
    let mut v = Vec::with_capacity(64);
    put_str(&mut v, &m.path);
    let mut tags = String::from(",");
    for a in &m.args {
        tags.push(match a {
            Arg::I(_) => 'i',
            Arg::F(_) => 'f',
            Arg::S(_) => 's',
            Arg::H(_) => 'h',
            Arg::D(_) => 'd',
            Arg::B(_) => 'b',
            Arg::T => 'T',
            Arg::False => 'F',
            Arg::Nil => 'N',
        });
    }
    put_str(&mut v, &tags);
    for a in &m.args {
        match a {
            Arg::I(i) => v.extend_from_slice(&i.to_be_bytes()),
            Arg::F(f) => v.extend_from_slice(&f.to_be_bytes()),
            Arg::S(s) => put_str(&mut v, s),
            Arg::H(h) => v.extend_from_slice(&h.to_be_bytes()),
            Arg::D(d) => v.extend_from_slice(&d.to_be_bytes()),
            Arg::B(b) => {
                v.extend_from_slice(&(b.len() as i32).to_be_bytes());
                v.extend_from_slice(b);
                pad(&mut v);
            }
            Arg::T | Arg::False | Arg::Nil => {}
        }
    }
    v
}

struct Rd<'a> {
    b: &'a [u8],
    at: usize,
}

impl Rd<'_> {
    fn str(&mut self) -> Option<String> {
        let rest = self.b.get(self.at..)?;
        let n = rest.iter().position(|&c| c == 0)?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.at += (n + 4) & !3;
        Some(s)
    }
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let s = self.b.get(self.at..self.at + N)?;
        self.at += N;
        s.try_into().ok()
    }
}

/// Decode a packet: one message, or a bundle's messages (recursively).
pub fn decode(b: &[u8]) -> Vec<Msg> {
    let mut out = Vec::new();
    decode_into(b, &mut out, 0);
    out
}

fn decode_into(b: &[u8], out: &mut Vec<Msg>, depth: u32) {
    if depth > 8 {
        return;
    }
    if b.starts_with(b"#bundle\0") {
        let mut at = 16; // "#bundle\0" + timetag
        while at + 4 <= b.len() {
            let n = i32::from_be_bytes(b[at..at + 4].try_into().unwrap()).max(0) as usize;
            at += 4;
            if at + n > b.len() {
                break;
            }
            decode_into(&b[at..at + n], out, depth + 1);
            at += n;
        }
        return;
    }
    let mut r = Rd { b, at: 0 };
    let Some(path) = r.str() else { return };
    if !path.starts_with('/') {
        return;
    }
    let tags = r.str().unwrap_or_else(|| ",".into());
    let mut args = Vec::new();
    for t in tags.chars().skip(1) {
        let a = match t {
            'i' => r.take::<4>().map(|x| Arg::I(i32::from_be_bytes(x))),
            'f' => r.take::<4>().map(|x| Arg::F(f32::from_be_bytes(x))),
            's' | 'S' => r.str().map(Arg::S),
            'h' | 't' => r.take::<8>().map(|x| Arg::H(i64::from_be_bytes(x))),
            'd' => r.take::<8>().map(|x| Arg::D(f64::from_be_bytes(x))),
            'b' => r.take::<4>().and_then(|x| {
                let n = i32::from_be_bytes(x).max(0) as usize;
                let d = r.b.get(r.at..r.at + n)?.to_vec();
                r.at += (n + 3) & !3;
                Some(Arg::B(d))
            }),
            'T' => Some(Arg::T),
            'F' => Some(Arg::False),
            'N' | 'I' => Some(Arg::Nil),
            'c' | 'r' | 'm' => r.take::<4>().map(|x| Arg::I(i32::from_be_bytes(x))),
            _ => None,
        };
        match a {
            Some(a) => args.push(a),
            None => return, // malformed: drop it
        }
    }
    out.push(Msg { path, args });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let m = Msg::new(
            "/nsm/server/announce",
            vec![Arg::S("Ardour".into()), Arg::S(":dirty:".into()), Arg::S("ardour8".into()), Arg::I(1), Arg::I(2), Arg::I(4242)],
        );
        let b = encode(&m);
        assert_eq!(b.len() % 4, 0);
        assert_eq!(decode(&b), vec![m]);
        let f = Msg::new("/nsm/client/progress", vec![Arg::F(0.5), Arg::B(vec![1, 2, 3]), Arg::T]);
        assert_eq!(decode(&encode(&f)), vec![f]);
    }

    #[test]
    fn bundle() {
        let a = encode(&Msg::new("/a", vec![Arg::I(1)]));
        let mut b = b"#bundle\0".to_vec();
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        b.extend_from_slice(&(a.len() as i32).to_be_bytes());
        b.extend_from_slice(&a);
        assert_eq!(decode(&b), vec![Msg::new("/a", vec![Arg::I(1)])]);
    }

    #[test]
    fn garbage() {
        assert!(decode(b"no slash\0\0\0\0").is_empty());
        assert!(decode(&[0xff; 3]).is_empty());
        let mut b = encode(&Msg::new("/x", vec![Arg::S("abc".into())]));
        b.truncate(b.len() - 4);
        assert!(decode(&b).is_empty());
    }
}
