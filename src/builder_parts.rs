// Included from builder.rs. Continues the builder: ::DataSpace files, $OBJINST,
// binary TOC/index, and the compile orchestration.

fn build_namelist() -> Vec<u8> {
    let mut b = Buf::new();
    b.u16(0);
    b.u16(2);
    for name in ["Uncompressed", "MSCompressed"] {
        b.u16(name.len() as u16);
        for c in name.bytes() {
            b.u16(c as u16);
        }
        b.u16(0);
    }
    let words = (b.len() / 2) as u16;
    b.v[0] = words as u8;
    b.v[1] = (words >> 8) as u8;
    b.v
}

fn build_control_data() -> Vec<u8> {
    let mut b = Buf::new();
    b.u32(6);
    b.raw(b"LZXC");
    b.u32(2);
    b.u32(2);
    b.u32(2);
    b.u32(1);
    b.u32(0);
    b.u32(0);
    b.v
}

fn build_reset_table(uncompressed: u64, compressed: u64, frame_starts: &[u64]) -> Vec<u8> {
    let mut b = Buf::new();
    b.u32(2);
    b.u32(frame_starts.len() as u32);
    b.u32(8);
    b.u32(0x28);
    b.u64(uncompressed);
    b.u64(compressed);
    b.u64(0x8000);
    for &off in frame_starts {
        b.u64(off);
    }
    b.v
}

fn build_transform_list() -> Vec<u8> {
    let g = b"{7FC28940-9D31-11D0-9B27-00A0C91E9C7C}";
    let mut b = Buf::new();
    for &c in &g[..19] {
        b.u16(c as u16);
    }
    b.v
}

fn build_subsets(subsets: &[String]) -> Vec<u8> {
    if subsets.is_empty() {
        return Vec::new();
    }
    let mut b = Buf::new();
    b.u16(0);
    b.u16((subsets.len() * 12) as u16);
    b.zeros(subsets.len() * 12);
    b.v
}

fn build_objinst(cp: u32, lcid: u32) -> Vec<u8> {
    let g_word_breaker = [0x9A, 0x56, 0x00, 0xC0, 0x4F, 0xB6, 0x8B, 0xF7];
    let g_stemmer = [0x9A, 0x61, 0x00, 0xC0, 0x4F, 0xB6, 0x8B, 0xF7];
    let g_system_sort = [0x9A, 0x56, 0x00, 0xC0, 0x4F, 0xB6, 0x8B, 0x66];
    let mut b = Buf::new();
    b.u32(0x04000000);
    b.u32(2);
    b.u32(24);
    b.u32(2691);
    b.u32(2715);
    b.u32(36);
    b.guid(0x4662DAAF, 0xD393, 0x11D0, &g_word_breaker);
    b.u32(0x04000000);
    b.u32(11);
    b.u32(cp);
    b.u32(lcid);
    b.u32(0);
    b.u32(0);
    b.u32(0x00145555);
    b.u32(0x00000A0F);
    b.u16(0x0100);
    b.u32(0x00030005);
    b.zeros(6 * 4);
    b.u16(0);
    for row in OBJINST_CHAR_TABLE.iter() {
        b.raw(row);
    }
    b.u32(0xE66561C6);
    b.u32(0x73DF6561);
    b.u32(0x656F8C73);
    b.u16(0x6F9C);
    b.u8(0x65);
    b.guid(0x8FA0D5A8, 0xDEDF, 0x11D0, &g_stemmer);
    b.u32(0x04000000);
    b.u32(1);
    b.u32(cp);
    b.u32(lcid);
    b.u32(0);
    b.guid(0x4662DAB0, 0xD393, 0x11D0, &g_system_sort);
    b.u32(666);
    b.u32(cp);
    b.u32(lcid);
    b.u32(10031);
    b.u32(0);
    b.v
}

fn sys_entry(b: &mut Buf, code: u16, value: u32) {
    b.u16(code);
    b.u16(4);
    b.u32(value);
}

// ---------------- binary TOC (#TOCIDX) ----------------

fn patch32(v: &mut [u8], off: usize, value: u32) {
    v[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

fn build_tocidx(toc: &SiteMap, strings: &mut Strings, urls: &mut Urls, topics: &mut Topics, cp: u32) -> Vec<u8> {
    struct Group<'a> {
        items: &'a [SiteMapItem],
        parent_pos: u32,
        has_parent: bool,
    }
    let mut info = Buf::new();
    let mut topic_dwords = Buf::new();
    let mut toc_entries: Vec<(u32, u32)> = Vec::new();
    let mut local_seq = 0u32;

    let mut level: Vec<Group> = vec![Group { items: &toc.items, parent_pos: 0, has_parent: false }];
    while !level.is_empty() {
        let mut next: Vec<Group> = Vec::new();
        for g in &level {
            let n = g.items.len();
            for (j, item) in g.items.iter().enumerate() {
                let local = slashes(&item.param("local"));
                let has_children = !item.children.is_empty();
                let props = (if has_children { 4u32 } else { 0 }) | (if local.is_empty() { 0 } else { 8u32 });
                let pos = info.len() as u32;
                if j == 0 && g.has_parent {
                    patch32(&mut info.v, g.parent_pos as usize + 0x14, 4096 + pos);
                }
                let topics_or_strings;
                if !local.is_empty() {
                    let mut t = topics.find(&local);
                    if t < 0 {
                        t = topics.add(strings, urls, &enc_cp(&item.param("name"), cp), &local, 2) as i32;
                    }
                    topics.patch_toc_offset(t as u32, 4096 + pos);
                    topics_or_strings = t as u32;
                    topic_dwords.u32(t as u32);
                    toc_entries.push((4096 + pos, t as u32));
                    local_seq += 1;
                } else {
                    topics_or_strings = strings.add_str(&item.param("name"), cp);
                }
                info.u16(0);
                info.u16(local_seq as u16);
                info.u32(props);
                info.u32(topics_or_strings);
                info.u32(if g.has_parent { 4096 + g.parent_pos } else { 0 });
                let last_sibling = j + 1 == n;
                info.u32(if last_sibling { 0 } else { 4096 + pos + if has_children { 28 } else { 20 } });
                if has_children {
                    info.u32(0);
                    info.u32(0);
                    next.push(Group { items: &item.children, parent_pos: pos, has_parent: true });
                }
            }
        }
        level = next;
    }

    let topics_offset = 4096 + info.len() as u32;
    let entries_offset = topics_offset + topic_dwords.len() as u32;
    let mut out = Buf::new();
    out.u32(4096);
    out.u32(entries_offset);
    out.u32(local_seq);
    out.u32(topics_offset);
    out.zeros(4096 - out.len());
    out.raw(&info.v);
    out.raw(&topic_dwords.v);
    for (i, &(page_book, topic)) in toc_entries.iter().enumerate() {
        out.u32(page_book);
        out.u32(0x29A + i as u32);
        out.u32(topics_offset + i as u32 * 4);
        out.u32(topic);
    }
    out.v
}

// ---------------- binary index ($WWKeywordLinks / $WWAssociativeLinks) ----------------

#[derive(Default)]
struct BinIndexFiles {
    btree: Vec<u8>,
    data: Vec<u8>,
    map: Vec<u8>,
    property: Vec<u8>,
}

fn assemble_btree(entries: &[Vec<u8>], lcid: u32, codepage: u32) -> BinIndexFiles {
    const BS: usize = 2048;
    let mut list_blocks: Vec<Vec<u8>> = Vec::new();
    let mut block_entry_counts: Vec<u32> = Vec::new();
    let mut block_first_idx: Vec<Vec<u8>> = Vec::new();
    let mut entries_before: Vec<u32> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut cur_count = 0u32;
    let mut done_entries = 0u32;
    let flush = |cur: &mut Vec<u8>,
                     cur_count: &mut u32,
                     list_blocks: &mut Vec<Vec<u8>>,
                     block_entry_counts: &mut Vec<u32>,
                     entries_before: &mut Vec<u32>,
                     done_entries: &mut u32| {
        entries_before.push(*done_entries);
        *done_entries += *cur_count;
        list_blocks.push(std::mem::take(cur));
        block_entry_counts.push(*cur_count);
        *cur_count = 0;
    };
    for e in entries {
        if e.len() > BS - 12 {
            continue;
        }
        if 12 + cur.len() + e.len() >= BS {
            flush(&mut cur, &mut cur_count, &mut list_blocks, &mut block_entry_counts, &mut entries_before, &mut done_entries);
        }
        if cur.is_empty() {
            let mut idx: Vec<u8> = e[..e.len() - 8].to_vec();
            let child = list_blocks.len() as u32;
            for k in 0..4 {
                idx.push((child >> (k * 8)) as u8);
            }
            block_first_idx.push(idx);
        }
        cur.extend_from_slice(e);
        cur_count += 1;
    }
    if !cur.is_empty() || list_blocks.is_empty() {
        flush(&mut cur, &mut cur_count, &mut list_blocks, &mut block_entry_counts, &mut entries_before, &mut done_entries);
    }
    let num_list = list_blocks.len() as u32;

    // index levels
    let mut levels: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut level_counts: Vec<Vec<u32>> = Vec::new();
    let mut level_first_child: Vec<Vec<u32>> = Vec::new();
    let mut cur_idx_forms = block_first_idx;
    let mut level_base = 0u32;
    while num_list > 1 {
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        let mut first_child: Vec<u32> = Vec::new();
        let mut next_forms: Vec<Vec<u8>> = Vec::new();
        let mut blk: Vec<u8> = Vec::new();
        let mut cnt = 0u32;
        let this_level_base = level_base + cur_idx_forms.len() as u32;
        for (i, e) in cur_idx_forms.iter_mut().enumerate() {
            let child = level_base + i as u32;
            let l = e.len();
            for k in 0..4 {
                e[l - 4 + k] = (child >> (k * 8)) as u8;
            }
            if 8 + blk.len() + e.len() >= BS {
                blocks.push(std::mem::take(&mut blk));
                counts.push(cnt);
                cnt = 0;
            }
            if blk.is_empty() {
                first_child.push(child);
                let mut up = e.clone();
                let my_nr = this_level_base + blocks.len() as u32;
                let lu = up.len();
                for k in 0..4 {
                    up[lu - 4 + k] = (my_nr >> (k * 8)) as u8;
                }
                next_forms.push(up);
            }
            blk.extend_from_slice(e);
            cnt += 1;
        }
        if !blk.is_empty() {
            blocks.push(blk);
            counts.push(cnt);
        }
        levels.push(blocks);
        level_counts.push(counts);
        level_first_child.push(first_child);
        level_base = this_level_base;
        cur_idx_forms = next_forms;
        if cur_idx_forms.len() <= 1 {
            break;
        }
    }

    let mut total_blocks = num_list;
    for lv in &levels {
        total_blocks += lv.len() as u32;
    }

    let mut bt = Buf::new();
    bt.u8(0x3B);
    bt.u8(0x29);
    bt.u16(2);
    bt.u16(BS as u16);
    bt.raw(b"X44\0\0\0\0\0\0\0\0\0\0\0\0\0");
    bt.u32(0);
    bt.u32(num_list - 1);
    bt.u32(total_blocks - 1);
    bt.i32(-1);
    bt.u32(total_blocks);
    bt.u16((1 + levels.len()) as u16);
    bt.u32(entries.len() as u32);
    bt.u32(codepage);
    bt.u32(lcid);
    bt.u32(1);
    bt.u32(10031);
    bt.u32(0);
    bt.u32(0);
    bt.u32(0);
    for i in 0..num_list as usize {
        let mut h = Buf::new();
        h.u16((BS - 12 - list_blocks[i].len()) as u16);
        h.u16(block_entry_counts[i] as u16);
        h.i32(if i == 0 { -1 } else { i as i32 - 1 });
        h.i32(if i as u32 + 1 == num_list { -1 } else { i as i32 + 1 });
        bt.raw(&h.v);
        bt.raw(&list_blocks[i]);
        bt.zeros(BS - 12 - list_blocks[i].len());
    }
    for lv in 0..levels.len() {
        for b in 0..levels[lv].len() {
            let mut h = Buf::new();
            h.u16((BS - 8 - levels[lv][b].len()) as u16);
            h.u16(level_counts[lv][b] as u16);
            h.u32(level_first_child[lv][b]);
            bt.raw(&h.v);
            bt.raw(&levels[lv][b]);
            bt.zeros(BS - 8 - levels[lv][b].len());
        }
    }

    let mut out = BinIndexFiles { btree: bt.v, ..Default::default() };
    let mut d = Buf::new();
    let data_entry = [0u8, 0, 0, 0, 5, 0, 0, 0, 0x80, 0, 0, 0, 0];
    for _ in entries {
        d.raw(&data_entry);
    }
    out.data = d.v;
    let mut m = Buf::new();
    m.u16(0);
    for i in 0..num_list {
        m.u32(entries_before[i as usize]);
        m.u32(i);
    }
    out.map = m.v;
    let mut pr = Buf::new();
    pr.u32(0);
    if !entries.is_empty() {
        pr.u32(0);
        pr.u32(0);
        pr.u32(0xC);
        pr.u32(1);
        pr.u32(1);
        pr.u32(0);
        pr.u32(0);
    }
    out.property = pr.v;
    out
}

#[derive(Default, Clone)]
struct KeywordEntry {
    display: String,
    sort_key: String,
    depth: u16,
    char_index: u32,
    see_also: bool,
    see_also_target: String,
    topic_ids: Vec<u32>,
}

#[derive(Default)]
struct BTreeBuilder {
    cp: u32,
    entries: Vec<KeywordEntry>,
    flat: BTreeMap<String, KeywordEntry>,
}

impl BTreeBuilder {
    fn new(cp: u32) -> Self {
        BTreeBuilder { cp, ..Default::default() }
    }
    fn empty(&self) -> bool {
        self.entries.is_empty() && self.flat.is_empty()
    }
    fn add_sitemap(&mut self, sm: &SiteMap, strings: &mut Strings, urls: &mut Urls, topics: &mut Topics) {
        let mut top: Vec<&SiteMapItem> = sm.items.iter().collect();
        top.sort_by(|a, b| lower(&a.param("name")).cmp(&lower(&b.param("name"))));
        for it in top {
            self.flatten(it, &it.param("name"), 0, 0, strings, urls, topics);
        }
    }
    fn add_keyword(&mut self, word: &str, topic_id: u32) {
        let key = lower(word);
        let e = self.flat.entry(key.clone()).or_default();
        if e.display.is_empty() {
            e.display = word.to_string();
            e.sort_key = key;
        }
        if !e.topic_ids.contains(&topic_id) {
            e.topic_ids.push(topic_id);
        }
    }
    fn flatten(&mut self, item: &SiteMapItem, path: &str, char_index: u32, depth: u16, strings: &mut Strings, urls: &mut Urls, topics: &mut Topics) {
        let mut e = KeywordEntry {
            display: path.to_string(),
            sort_key: lower(path),
            depth,
            char_index: if depth == 0 { 0 } else { char_index },
            ..Default::default()
        };
        let see_also = item.param("see also");
        if !see_also.is_empty() {
            e.see_also = true;
            e.see_also_target = see_also;
        } else {
            let mut last_name = item.param("name");
            let mut first_name = true;
            for (n, v) in &item.params {
                if n == "name" {
                    if !first_name {
                        last_name = v.clone();
                    }
                    first_name = false;
                } else if n == "local" {
                    let local = slashes(v);
                    let mut idx = topics.find(&local);
                    if idx < 0 {
                        idx = topics.add(strings, urls, &enc_cp(&last_name, self.cp), &local, -1) as i32;
                    }
                    e.topic_ids.push(idx as u32);
                }
            }
        }
        self.entries.push(e);
        for child in &item.children {
            let cp = format!("{}, {}", path, child.param("name"));
            self.flatten(child, &cp, path.len() as u32 + 2, depth + 1, strings, urls, topics);
        }
    }
    fn serialize(&self, e: &KeywordEntry, seq: u32) -> Vec<u8> {
        let mut b = Buf::new();
        append_utf16le(&mut b.v, &decode_auto(&e.display, self.cp));
        b.u16(0);
        b.u16(if e.see_also { 2 } else { 0 });
        b.u16(e.depth);
        b.u32(e.char_index);
        b.u32(0);
        if e.see_also {
            b.u32(1);
            append_utf16le(&mut b.v, &decode_auto(&e.see_also_target, self.cp));
            b.u16(0);
        } else {
            b.u32(e.topic_ids.len() as u32);
            for &t in &e.topic_ids {
                b.u32(t);
            }
        }
        b.u32(1);
        b.u32(seq * 13);
        b.v
    }
    fn assemble(&mut self, lcid: u32) -> BinIndexFiles {
        let flat = std::mem::take(&mut self.flat);
        for (_, v) in flat {
            self.entries.push(v);
        }
        self.entries.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
        let mut blobs: Vec<Vec<u8>> = Vec::with_capacity(self.entries.len());
        for (i, e) in self.entries.iter().enumerate() {
            blobs.push(self.serialize(e, i as u32));
        }
        assemble_btree(&blobs, lcid, self.cp)
    }
}

// ---------------- #IDXHDR ----------------

fn build_idxhdr(topics: &Topics, toc: &SiteMap, strings: &mut Strings, merge_files: &[String], cp: u32) -> Vec<u8> {
    let prop = |name: &str| -> String {
        toc.properties.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or_default()
    };
    let str_or_none = |s: &mut Strings, v: String, none: u32| if v.is_empty() { none } else { s.add_str(&v, cp) };
    let color_or_none = |v: String| if v.is_empty() { 0xFFFFFFFFu32 } else { parse_int(&v) };
    let folder = lower(&prop("imagetype")) == "folder";

    let mut b = Buf::new();
    b.raw(b"T#SM");
    b.u32(0);
    b.u32(1);
    b.u32(topics.count());
    b.u32(0);
    b.u32(str_or_none(strings, prop("imagelist"), 0xFFFFFFFF));
    b.u32(0);
    b.u32(if folder { 1 } else { 0 });
    b.u32(color_or_none(prop("background")));
    b.u32(color_or_none(prop("foreground")));
    b.u32(str_or_none(strings, prop("font"), 0xFFFFFFFF));
    b.u32(0xFFFFFFFF);
    b.u32(0);
    b.u32(0xFFFFFFFF);
    b.u32(str_or_none(strings, prop("framename"), 0));
    b.u32(str_or_none(strings, prop("windowname"), 0xFFFFFFFF));
    b.u32(0);
    b.u32(1);
    b.u32(merge_files.len() as u32);
    b.u32(if merge_files.is_empty() { 0 } else { 1 });
    for mf in merge_files {
        b.u32(strings.add(mf.as_bytes()));
    }
    for _ in merge_files.len()..1004 {
        b.u32(0);
    }
    b.zeros(4096 - b.len());
    b.v
}

fn build_windows(windows: &[Window], strings: &mut Strings, cp: u32) -> Vec<u8> {
    if windows.is_empty() {
        return Vec::new();
    }
    let mut b = Buf::new();
    b.u32(windows.len() as u32);
    b.u32(196);
    for w in windows {
        b.u32(196);
        b.u32(0);
        b.u32(strings.add_str(&w.typ, cp));
        b.u32(w.valid_flags);
        b.u32(w.nav_style);
        b.u32(strings.add_str(&w.caption, cp));
        b.u32(w.styles);
        b.u32(w.ex_styles);
        for k in 0..4 {
            b.i32(w.rect[k]);
        }
        b.u32(w.show_state);
        b.zeros(6 * 4);
        b.u32(w.nav_width);
        b.zeros(4 * 4);
        b.u32(strings.add(w.toc.as_bytes()));
        b.u32(strings.add(w.index.as_bytes()));
        b.u32(strings.add(w.default_file.as_bytes()));
        b.u32(strings.add(w.home.as_bytes()));
        b.u32(w.buttons);
        b.u32(w.nav_closed);
        b.u32(w.nav_default);
        b.u32(w.nav_pos);
        b.u32(w.notify_id);
        b.zeros(5 * 4);
        b.u32(0);
        b.u32(strings.add_str(&w.jump1_text, cp));
        b.u32(strings.add_str(&w.jump2_text, cp));
        b.u32(strings.add(w.jump1_file.as_bytes()));
        b.u32(strings.add(w.jump2_file.as_bytes()));
        b.zeros(4 * 4);
        b.u32(0);
        b.u32(0);
    }
    b.v
}

#[allow(clippy::too_many_arguments)]
fn build_system(
    p: &Project,
    lcid: u32,
    cp: u32,
    hhc: &str,
    hhk: &str,
    default_window: &str,
    binary_toc: bool,
    has_klinks: bool,
    has_alinks: bool,
    idxhdr: &[u8],
    fts: bool,
) -> Vec<u8> {
    let mut b = Buf::new();
    b.u32(3);
    b.u16(10);
    b.u16(4);
    b.u32(0);
    sys_entry_str(&mut b, 9, concat!("rustchm ", env!("CARGO_PKG_VERSION")));
    b.u16(4);
    b.u16(36);
    b.u32(lcid);
    b.u32(0);
    b.u32(if fts { 1 } else { 0 });
    b.u32(if has_klinks { 1 } else { 0 });
    b.u32(if has_alinks { 1 } else { 0 });
    b.u64(0);
    b.u32(0);
    b.u32(0);
    let _ = cp;
    let def_topic = slashes(&p.opt("default topic"));
    if !def_topic.is_empty() {
        sys_entry_str(&mut b, 2, &def_topic);
    }
    if !p.opt("title").is_empty() {
        sys_entry_str(&mut b, 3, &p.opt("title"));
    }
    if !p.opt("default font").is_empty() {
        sys_entry_str(&mut b, 16, &p.opt("default font"));
    }
    if !hhc.is_empty() {
        sys_entry_str(&mut b, 0, hhc);
    }
    if !hhk.is_empty() {
        sys_entry_str(&mut b, 1, hhk);
    }
    if !default_window.is_empty() {
        sys_entry_str(&mut b, 5, default_window);
    }
    if !idxhdr.is_empty() {
        sys_entry(&mut b, 7, 0);
    }
    if binary_toc {
        sys_entry(&mut b, 11, 0);
    }
    if !p.info_types.is_empty() {
        sys_entry(&mut b, 12, p.info_types.len() as u32);
    }
    if !idxhdr.is_empty() {
        b.u16(13);
        b.u16(idxhdr.len() as u16);
        b.raw(idxhdr);
    }
    b.v
}

// ---------------- DBCS dummy alink data entry helper ----------------
fn dummy_alink_property() -> Vec<u8> {
    let mut b = Buf::new();
    b.u32(0);
    b.v
}

struct Loaded {
    rel: String,
    data: Vec<u8>,
}

fn gather_files(
    p: &Project,
    hhc: &str,
    hhk: &str,
) -> Result<(Vec<Loaded>, Option<SiteMap>, Option<SiteMap>), String> {
    let mut queue: Vec<(String, bool)> = Vec::new();
    let mut enqueued: HashSet<String> = HashSet::new();
    let push = |q: &mut Vec<(String, bool)>, set: &mut HashSet<String>, rel: String, explicit: bool| {
        if rel.is_empty() {
            return;
        }
        if set.insert(lower(&rel)) {
            q.push((rel, explicit));
        }
    };
    for f in &p.files {
        push(&mut queue, &mut enqueued, f.clone(), true);
    }
    let mut loaded: Vec<Loaded> = Vec::new();
    let mut toc = None;
    let mut index = None;
    let mut qi = 0;
    while qi < queue.len() {
        let (rel, explicit) = queue[qi].clone();
        qi += 1;
        let data = match std::fs::read(format!("{}{}", p.dir, rel)) {
            Ok(d) => d,
            Err(_) => {
                if explicit {
                    return Err(format!("cannot read file: {}{}", p.dir, rel));
                }
                eprintln!("rustchm: warning: referenced file not found: {rel}");
                continue;
            }
        };
        let lrel = lower(&rel);
        let is_hhc = lrel == lower(hhc);
        let is_hhk = !hhk.is_empty() && lrel == lower(hhk);
        if is_hhc || is_hhk {
            let sm = parse_sitemap(&data);
            let mut locals = Vec::new();
            sm.collect_locals(&mut locals);
            for l in locals {
                push(&mut queue, &mut enqueued, resolve_ref("", &l), false);
            }
            if is_hhc {
                toc = Some(sm);
            } else {
                index = Some(sm);
            }
        } else if is_html_name(&rel) {
            let mut refs = Vec::new();
            extract_refs(&data, &mut refs);
            let base = dir_of(&rel);
            for r in refs {
                push(&mut queue, &mut enqueued, resolve_ref(&base, &r), false);
            }
        }
        loaded.push(Loaded { rel, data });
    }
    Ok((loaded, toc, index))
}

pub fn compile_project(hhp_path: &str, out_override: &str) -> Result<(Stats, String), String> {
    let mut p = parse_hhp(hhp_path).map_err(|e| format!("cannot read project: {e}"))?;
    let lcid = parse_lcid(&p.opt("language"));
    let cp = if p.opt("charset").is_empty() {
        codepage_for_lcid(lcid)
    } else {
        parse_int(&p.opt("charset"))
    };
    let hhc = slashes(&p.opt("contents file"));
    let hhk = slashes(&p.opt("index file"));

    let ensure = |files: &mut Vec<String>, f: &str| {
        if !f.is_empty() && !files.iter().any(|e| lower(e) == lower(f)) {
            files.push(f.to_string());
        }
    };
    ensure(&mut p.files, &hhc);
    ensure(&mut p.files, &hhk);
    if p.files.is_empty() {
        return Err("project has no [FILES]".into());
    }

    let (loaded, toc, index) = gather_files(&p, &hhc, &hhk)?;
    let have_toc = toc.is_some();
    let have_index = index.is_some();
    let toc = toc.unwrap_or_default();
    let index = index.unwrap_or_default();

    let windows: Vec<Window> = p.window_lines.iter().map(|l| parse_window_line(l)).collect();
    let mut default_window = p.opt("default window");
    if default_window.is_empty() && !windows.is_empty() {
        default_window = windows[0].typ.clone();
    }

    let mut strings = Strings::new();
    let mut urls = Urls::new();
    let mut topics = Topics::new();
    let mut fts = FtsIndexer::new();
    let mut klinks = BTreeBuilder::new(cp);
    let mut alinks = BTreeBuilder::new(cp);

    let mut entries: Vec<DirEntry> = Vec::new();
    let mut section1 = Buf::new();

    let fts_option = p.opt_yes("full-text search") || p.opt_yes("full text search");
    let ivb = build_ivb(&p, &mut strings);
    if !ivb.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#IVB", &ivb);
    }
    if fts_option {
        add_sec(&mut entries, &mut section1, 1, "/$OBJINST", &build_objinst(cp, lcid));
    }

    for lf in &loaded {
        add_sec(&mut entries, &mut section1, 1, &format!("/{}", lf.rel), &lf.data);
        if is_html_name(&lf.rel) {
            let topic_idx = topics.add(&mut strings, &mut urls, &extract_title_s(&lf.data, cp), &format!("/{}", lf.rel), -1);
            if fts_option {
                fts.index_file(&lf.data, topic_idx);
            }
            let lo = scan_link_objects(&lf.data);
            for kw in lo.keywords {
                klinks.add_keyword(&kw, topic_idx);
            }
            for al in lo.alinks {
                alinks.add_keyword(&al, topic_idx);
            }
        }
    }
    let file_count = loaded.len();

    if !hhc.is_empty() {
        topics.add(&mut strings, &mut urls, b"", &hhc, 2);
    }
    if !hhk.is_empty() {
        topics.add(&mut strings, &mut urls, b"", &hhk, 2);
    }

    let binary_toc = p.opt_yes("binary toc") && have_toc;
    let binary_index = p.opt_yes("binary index") && have_index;
    let toc_idx = if binary_toc {
        build_tocidx(&toc, &mut strings, &mut urls, &mut topics, cp)
    } else {
        Vec::new()
    };
    if binary_index {
        klinks.add_sitemap(&index, &mut strings, &mut urls, &mut topics);
    }
    let have_klinks = !klinks.empty();
    let have_alinks = !alinks.empty();
    let k_files = if have_klinks { klinks.assemble(lcid) } else { BinIndexFiles::default() };
    let a_files = if have_alinks { alinks.assemble(lcid) } else { BinIndexFiles::default() };
    let idxhdr = if binary_index || !p.merge_files.is_empty() {
        build_idxhdr(&topics, &toc, &mut strings, &p.merge_files, cp)
    } else {
        Vec::new()
    };

    if !topics.buf.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#TOPICS", &topics.buf.v);
    }
    if !urls.urlstr.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#URLSTR", &urls.urlstr.v);
    }
    if !urls.urltbl.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#URLTBL", &urls.urltbl.v);
    }
    if !toc_idx.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#TOCIDX", &toc_idx);
    }
    if have_klinks {
        add_sec(&mut entries, &mut section1, 1, "/$WWKeywordLinks/BTree", &k_files.btree);
        add_sec(&mut entries, &mut section1, 1, "/$WWKeywordLinks/Data", &k_files.data);
        add_sec(&mut entries, &mut section1, 1, "/$WWKeywordLinks/Map", &k_files.map);
        add_sec(&mut entries, &mut section1, 1, "/$WWKeywordLinks/Property", &k_files.property);
    }
    if have_alinks {
        add_sec(&mut entries, &mut section1, 1, "/$WWAssociativeLinks/BTree", &a_files.btree);
        add_sec(&mut entries, &mut section1, 1, "/$WWAssociativeLinks/Data", &a_files.data);
        add_sec(&mut entries, &mut section1, 1, "/$WWAssociativeLinks/Map", &a_files.map);
        add_sec(&mut entries, &mut section1, 1, "/$WWAssociativeLinks/Property", &a_files.property);
    } else if have_klinks {
        add_sec(&mut entries, &mut section1, 1, "/$WWAssociativeLinks/Property", &dummy_alink_property());
    }
    let windows_file = build_windows(&windows, &mut strings, cp);
    if !windows_file.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#WINDOWS", &windows_file);
    }
    let subsets_file = build_subsets(&p.subsets);
    if !subsets_file.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#SUBSETS", &subsets_file);
    }
    if !idxhdr.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/#IDXHDR", &idxhdr);
    }
    if strings.buf.is_empty() {
        strings.buf.u8(0);
    }
    add_sec(&mut entries, &mut section1, 1, "/#STRINGS", &strings.buf.v);
    let fifti = if fts_option && fts.has_data() {
        fts.build(lcid, cp)
    } else {
        Vec::new()
    };
    if !fifti.is_empty() {
        add_sec(&mut entries, &mut section1, 1, "/$FIftiMain", &fifti);
    }

    let uncompressed = section1.len() as u64;
    let lzx = lzx_compress(&section1.v);

    // ---- section 0 ----
    let mut section0 = Buf::new();
    entries.push(DirEntry { name: "/#ITBITS".into(), section: 0, offset: 0, size: 0 });
    add_sec(&mut entries, &mut section0, 0, "/#SYSTEM", &build_system(&p, lcid, cp, &hhc, &hhk, &default_window, binary_toc, have_klinks, have_alinks, &idxhdr, !fifti.is_empty()));
    add_sec(&mut entries, &mut section0, 0, "::DataSpace/NameList", &build_namelist());
    add_sec(&mut entries, &mut section0, 0, "::DataSpace/Storage/MSCompressed/ControlData", &build_control_data());
    {
        let mut span = Buf::new();
        span.u64(uncompressed);
        add_sec(&mut entries, &mut section0, 0, "::DataSpace/Storage/MSCompressed/SpanInfo", &span.v);
    }
    add_sec(&mut entries, &mut section0, 0, "::DataSpace/Storage/MSCompressed/Transform/List", &build_transform_list());
    add_sec(&mut entries, &mut section0, 0, "::DataSpace/Storage/MSCompressed/Transform/{7FC28940-9D31-11D0-9B27-00A0C91E9C7C}/InstanceData/ResetTable", &build_reset_table(uncompressed, lzx.data.len() as u64, &lzx.frame_starts));
    entries.push(DirEntry {
        name: "::DataSpace/Storage/MSCompressed/Content".into(),
        section: 0,
        offset: section0.len() as u64,
        size: lzx.data.len() as u64,
    });

    let _ = is_dbcs_codepage(cp);

    let out = if !out_override.is_empty() {
        out_override.to_string()
    } else {
        let compiled = slashes(&p.opt("compiled file"));
        if !compiled.is_empty() {
            format!("{}{}", p.dir, compiled)
        } else {
            let stem = Path::new(hhp_path).file_stem().unwrap().to_string_lossy();
            format!("{}{}.chm", p.dir, stem)
        }
    };
    let size = write_container(&out, lcid, entries, &section0.v, &lzx.data).map_err(|e| format!("write failed: {e}"))?;
    Ok((Stats { file_count, uncompressed, compressed: lzx.data.len() as u64, output: size }, out))
}

fn add_sec(entries: &mut Vec<DirEntry>, buf: &mut Buf, section: u32, name: &str, data: &[u8]) {
    entries.push(DirEntry { name: name.to_string(), section, offset: buf.len() as u64, size: data.len() as u64 });
    buf.raw(data);
}

fn build_ivb(p: &Project, strings: &mut Strings) -> Vec<u8> {
    let mut ctx: BTreeMap<u32, String> = BTreeMap::new();
    for (name, id) in &p.map_defs {
        if let Some((_, file)) = p.aliases.iter().find(|(n, _)| n == name) {
            ctx.insert(*id, file.clone());
        } else {
            eprintln!("rustchm: warning: [MAP] name has no [ALIAS] entry: {name}");
        }
    }
    if ctx.is_empty() {
        return Vec::new();
    }
    let mut b = Buf::new();
    b.u32((ctx.len() * 8) as u32);
    for (id, file) in &ctx {
        b.u32(*id);
        b.u32(strings.add(file.as_bytes()));
    }
    b.v
}

// ---------------- collection ----------------

pub struct CollectionMember {
    pub hhp: String,
    pub chm: String,
    pub is_master: bool,
    pub ok: bool,
    pub reused: bool,
    pub stats: Option<Stats>,
    pub err: String,
}

pub fn compile_collection(master_hhp: &str) -> Result<Vec<CollectionMember>, String> {
    let master = parse_hhp(master_hhp).map_err(|e| format!("cannot read project: {e}"))?;
    let dir = master.dir.clone();
    let file_exists = |path: &str| std::fs::metadata(path).is_ok();
    let stem = |chm: &str| -> String {
        let s = chm.rsplit('/').next().unwrap_or(chm);
        s.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_else(|| s.to_string())
    };
    let mut out: Vec<CollectionMember> = Vec::new();
    for mf in &master.merge_files {
        let name = stem(mf);
        let chm = format!("{dir}{name}.chm");
        let flat = format!("{dir}{name}.hhp");
        let nested = format!("{dir}{name}/{name}.hhp");
        let child_hhp = if file_exists(&flat) {
            flat
        } else if file_exists(&nested) {
            nested
        } else {
            String::new()
        };
        if child_hhp.is_empty() {
            let ok = file_exists(&chm);
            out.push(CollectionMember {
                hhp: String::new(),
                chm: chm.clone(),
                is_master: false,
                ok,
                reused: true,
                stats: None,
                err: if ok { String::new() } else { format!("no {name}.hhp and no prebuilt {name}.chm") },
            });
            continue;
        }
        match compile_project(&child_hhp, &chm) {
            Ok((stats, _)) => out.push(CollectionMember { hhp: child_hhp, chm, is_master: false, ok: true, reused: false, stats: Some(stats), err: String::new() }),
            Err(e) => out.push(CollectionMember { hhp: child_hhp, chm, is_master: false, ok: false, reused: false, stats: None, err: e }),
        }
    }
    match compile_project(master_hhp, "") {
        Ok((stats, used)) => out.push(CollectionMember { hhp: master_hhp.to_string(), chm: used, is_master: true, ok: true, reused: false, stats: Some(stats), err: String::new() }),
        Err(e) => out.push(CollectionMember { hhp: master_hhp.to_string(), chm: String::new(), is_master: true, ok: false, reused: false, stats: None, err: e }),
    }
    Ok(out)
}
