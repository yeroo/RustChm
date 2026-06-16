//! Parser for HTML Help "sitemap" files (.hhc / .hhk) and in-topic keyword/ALink
//! <object> controls. Port of FastChm's sitemap.cpp.

#[derive(Default, Clone)]
pub struct SiteMapItem {
    pub params: Vec<(String, String)>, // lower-cased names
    pub children: Vec<SiteMapItem>,
}

impl SiteMapItem {
    pub fn param(&self, name: &str) -> String {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

#[derive(Default)]
pub struct SiteMap {
    pub items: Vec<SiteMapItem>,
    pub properties: Vec<(String, String)>, // text/site properties
}

impl SiteMap {
    pub fn collect_locals(&self, out: &mut Vec<String>) {
        fn walk(items: &[SiteMapItem], out: &mut Vec<String>) {
            for it in items {
                for (n, v) in &it.params {
                    if n == "local" && !v.is_empty() {
                        out.push(v.clone());
                    }
                }
                walk(&it.children, out);
            }
        }
        walk(&self.items, out);
    }
}

#[derive(Default)]
pub struct LinkObjects {
    pub keywords: Vec<String>,
    pub alinks: Vec<String>,
}

fn lower(c: u8) -> u8 {
    if c.is_ascii_uppercase() {
        c + 32
    } else {
        c
    }
}

struct Tag {
    name: String, // lower-cased, leading '/' for close tags
    attrs: Vec<(String, String)>,
}

fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        if let Some(semi) = s[i..].find(';') {
            let semi = i + semi;
            if semi - i <= 10 {
                let ent = &s[i + 1..semi];
                match ent {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ if ent.starts_with('#') => {
                        let hex = ent.len() > 1 && (ent.as_bytes()[1] | 0x20) == b'x';
                        let num = if hex { &ent[2..] } else { &ent[1..] };
                        if let Ok(code) = u32::from_str_radix(num, if hex { 16 } else { 10 }) {
                            if code > 0 && code < 256 {
                                out.push(code as u8 as char);
                            }
                        }
                    }
                    _ => out.push_str(&s[i..semi + 1]),
                }
                i = semi + 1;
                continue;
            }
        }
        out.push('&');
        i += 1;
    }
    out
}

fn parse_tag(s: &[u8], i: &mut usize) -> Tag {
    let mut t = Tag { name: String::new(), attrs: Vec::new() };
    *i += 1; // '<'
    while *i < s.len() && (s[*i] == b'/' || !s[*i].is_ascii_whitespace()) {
        if s[*i] == b'>' {
            break;
        }
        t.name.push(lower(s[*i]) as char);
        *i += 1;
    }
    while *i < s.len() && s[*i] != b'>' {
        while *i < s.len() && s[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if *i >= s.len() || s[*i] == b'>' || s[*i] == b'/' {
            if *i < s.len() && s[*i] == b'/' {
                *i += 1;
            }
            continue;
        }
        let mut name = String::new();
        while *i < s.len() && s[*i] != b'=' && s[*i] != b'>' && !s[*i].is_ascii_whitespace() {
            name.push(lower(s[*i]) as char);
            *i += 1;
        }
        let mut value = String::new();
        while *i < s.len() && s[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if *i < s.len() && s[*i] == b'=' {
            *i += 1;
            while *i < s.len() && s[*i].is_ascii_whitespace() {
                *i += 1;
            }
            if *i < s.len() && (s[*i] == b'"' || s[*i] == b'\'') {
                let q = s[*i];
                *i += 1;
                while *i < s.len() && s[*i] != q {
                    value.push(s[*i] as char);
                    *i += 1;
                }
                if *i < s.len() {
                    *i += 1;
                }
            } else {
                while *i < s.len() && s[*i] != b'>' && !s[*i].is_ascii_whitespace() {
                    value.push(s[*i] as char);
                    *i += 1;
                }
            }
        }
        if !name.is_empty() {
            t.attrs.push((name, decode_entities(&value)));
        }
    }
    if *i < s.len() {
        *i += 1; // '>'
    }
    t
}

fn attr(t: &Tag, name: &str) -> String {
    t.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or_default()
}

pub fn parse_sitemap(data: &[u8]) -> SiteMap {
    let mut sm = SiteMap::default();
    let s = data;
    // stack of item-list paths via indices is awkward with ownership; build a tree
    // using an explicit stack of nodes.
    enum Mode {
        None,
        Sitemap,
        Properties,
    }
    // We assemble children bottom-up using a stack of Vec<SiteMapItem>.
    let mut stack: Vec<Vec<SiteMapItem>> = vec![Vec::new()];
    let mut object_mode = Mode::None;
    let mut pending = SiteMapItem::default();

    let mut i = 0;
    while i < s.len() {
        if s[i] != b'<' {
            i += 1;
            continue;
        }
        if s[i..].starts_with(b"<!--") {
            i = s[i..].windows(3).position(|w| w == b"-->").map(|x| i + x + 3).unwrap_or(s.len());
            continue;
        }
        let t = parse_tag(s, &mut i);
        match t.name.as_str() {
            "ul" => {
                // a new list nests under the last item of the current top list
                if stack.last().unwrap().is_empty() {
                    // no parent item yet — keep a sibling list at same level
                    stack.push(Vec::new());
                } else {
                    stack.push(Vec::new());
                }
            }
            "/ul" => {
                if stack.len() > 1 {
                    let children = stack.pop().unwrap();
                    if let Some(parent) = stack.last_mut().unwrap().last_mut() {
                        parent.children = children;
                    } else {
                        // orphan list: merge into parent level
                        stack.last_mut().unwrap().extend(children);
                    }
                }
            }
            "object" => {
                let ty = attr(&t, "type").to_ascii_lowercase();
                if ty == "text/sitemap" {
                    object_mode = Mode::Sitemap;
                    pending = SiteMapItem::default();
                } else if ty == "text/site properties" {
                    object_mode = Mode::Properties;
                }
            }
            "param" => {
                if !matches!(object_mode, Mode::None) {
                    let name = attr(&t, "name").to_ascii_lowercase();
                    let value = attr(&t, "value");
                    if !name.is_empty() {
                        match object_mode {
                            Mode::Sitemap => pending.params.push((name, value)),
                            Mode::Properties => sm.properties.push((name, value)),
                            Mode::None => {}
                        }
                    }
                }
            }
            "/object" => {
                if matches!(object_mode, Mode::Sitemap) {
                    stack.last_mut().unwrap().push(std::mem::take(&mut pending));
                }
                object_mode = Mode::None;
            }
            _ => {}
        }
    }
    sm.items = stack.into_iter().next().unwrap_or_default();
    sm
}

pub fn scan_link_objects(data: &[u8]) -> LinkObjects {
    let mut out = LinkObjects::default();
    let s = data;
    let mut in_object = false;
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'<' {
            i += 1;
            continue;
        }
        if s[i..].starts_with(b"<!--") {
            i = s[i..].windows(3).position(|w| w == b"-->").map(|x| i + x + 3).unwrap_or(s.len());
            continue;
        }
        let t = parse_tag(s, &mut i);
        match t.name.as_str() {
            "object" => in_object = true,
            "/object" => in_object = false,
            "param" if in_object => {
                let name = attr(&t, "name").to_ascii_lowercase();
                let value = attr(&t, "value");
                if !value.is_empty() {
                    if name == "keyword" {
                        out.keywords.push(value);
                    } else if name == "alink name" {
                        out.alinks.push(value);
                    }
                }
            }
            _ => {}
        }
    }
    out
}
