use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};


const COLOR_NAMES: &str = "orange, blue, red, white или #RRGGBB";

fn parse_color(name: &str) -> Option<((u8, u8, u8), u8)> {
    match name.to_lowercase().as_str() {
        "orange" => Some(((255, 90, 0), 202)),
        "blue" => Some(((40, 120, 255), 33)),
        "red" => Some(((235, 30, 40), 196)),
        "white" => Some(((240, 240, 240), 255)),
        hex => {
            let h = hex.strip_prefix('#')?;
            if h.len() != 6 {
                return None;
            }
            let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
            let (r, g, b) = (c(0)?, c(2)?, c(4)?);
            // ближайший цвет из куба 6×6×6 палитры xterm-256
            let q = |v: u8| ((v as u16 * 5 + 127) / 255) as u8;
            Some(((r, g, b), 16 + 36 * q(r) + 6 * q(g) + q(b)))
        }
    }
}

fn color_code((rgb, idx): ((u8, u8, u8), u8)) -> String {
    if env::var_os("NO_COLOR").is_some() {
        return String::new();
    }
    match env::var("COLORTERM").as_deref() {
        Ok("truecolor") | Ok("24bit") => format!("\x1b[1;38;2;{};{};{}m", rgb.0, rgb.1, rgb.2),
        _ => format!("\x1b[1;38;5;{idx}m"),
    }
}

fn reset() -> &'static str {
    if env::var_os("NO_COLOR").is_some() { "" } else { "\x1b[0m" }
}

fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| fs::read_to_string("/etc/hostname"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn username() -> String {
    // Сначала реальный uid из /proc, затем имя из /etc/passwd — так не обмануть подменой $USER.
    let uid = fs::read_to_string("/proc/self/status").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("Uid:"))
            .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
    });
    if let (Some(uid), Ok(passwd)) = (uid, fs::read_to_string("/etc/passwd")) {
        for line in passwd.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() > 2 && f[2] == uid {
                return f[0].to_string();
            }
        }
    }
    env::var("USER")
        .or_else(|_| env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn loadavg() -> String {
    fs::read_to_string("/proc/loadavg")
        .ok()
        .map(|s| {
            let v: Vec<&str> = s.split_whitespace().take(3).collect();
            v.join(" / ")
        })
        .unwrap_or_else(|| "? / ? / ?".into())
}

fn uptime() -> String {
    let secs = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    let (d, h, m, s) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60, secs % 60);
    if d > 0 {
        format!("{d}D {h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn short_layout(name: &str) -> String {
    let n = name.trim().trim_matches('\'').trim_matches('"');
    let lower = n.to_lowercase();
    let known = [
        ("english", "US"), ("russian", "RU"), ("ukrainian", "UA"), ("german", "DE"),
        ("french", "FR"), ("spanish", "ES"), ("italian", "IT"), ("polish", "PL"),
        ("belarusian", "BY"), ("kazakh", "KZ"), ("japanese", "JP"), ("chinese", "CN"),
    ];
    if n.len() > 3 {
        if lower.contains("(uk)") || lower.contains("united kingdom") {
            return "GB".into();
        }
        for (k, v) in known {
            if lower.contains(k) {
                return v.into();
            }
        }
    }
    let code = n.trim_start_matches("xkb:").split(|c| c == '(' || c == ':' || c == '+').next().unwrap_or(n);
    code.to_uppercase()
}

fn keyboard_layout() -> String {
    // Hyprland
    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        if let Some(out) = run("hyprctl", &["devices"]) {
            if let Some(l) = out.lines().find(|l| l.contains("active keymap:")) {
                return short_layout(l.split(':').nth(1).unwrap_or(""));
            }
        }
    }
    // Sway
    if env::var_os("SWAYSOCK").is_some() {
        if let Some(out) = run("swaymsg", &["-t", "get_inputs", "-r"]) {
            if let Some(i) = out.find("\"xkb_active_layout_name\"") {
                let rest = &out[i..];
                if let Some(v) = rest.split('"').nth(3) {
                    return short_layout(v);
                }
            }
        }
    }
    // KDE Plasma
    for q in ["qdbus6", "qdbus"] {
        if let (Some(idx), Some(list)) = (
            run(q, &["org.kde.keyboard", "/Layouts", "getLayout"]),
            run(q, &["org.kde.keyboard", "/Layouts", "getLayoutsList"]),
        ) {
            if let Ok(i) = idx.trim().parse::<usize>() {
                // формат: строки "us", "", "English (US)", ...  — берём каждую третью
                let codes: Vec<&str> = list.lines().step_by(3).collect();
                if let Some(c) = codes.get(i) {
                    return short_layout(c);
                }
            }
        }
    }
    // GNOME: первый элемент mru-sources — активный источник ввода
    if let Some(out) = run("gsettings", &["get", "org.gnome.desktop.input-sources", "mru-sources"]) {
        if let Some(v) = out.split('\'').nth(3) {
            return short_layout(v);
        }
    }
    // X11: xkb-switch показывает активную группу
    if let Some(out) = run("xkb-switch", &[]) {
        if !out.trim().is_empty() {
            return short_layout(&out);
        }
    }
    // X11: setxkbmap знает только список раскладок, берём первую
    if let Some(out) = run("setxkbmap", &["-query"]) {
        if let Some(l) = out.lines().find(|l| l.starts_with("layout:")) {
            return short_layout(l[7..].split(',').next().unwrap_or(""));
        }
    }
    // Системный конфиг (Debian/Ubuntu) и localectl
    if let Ok(s) = fs::read_to_string("/etc/default/keyboard") {
        if let Some(l) = s.lines().find(|l| l.starts_with("XKBLAYOUT=")) {
            return short_layout(l[10..].trim_matches('"').split(',').next().unwrap_or(""));
        }
    }
    if let Some(out) = run("localectl", &["status"]) {
        if let Some(l) = out.lines().find(|l| l.contains("X11 Layout:")) {
            return short_layout(l.split(':').nth(1).unwrap_or("").split(',').next().unwrap_or(""));
        }
    }
    "??".into()
}

fn tz_offset(now: i64) -> i64 {
    let path = env::var("TZ")
        .ok()
        .filter(|t| !t.is_empty())
        .map(|t| format!("/usr/share/zoneinfo/{}", t.trim_start_matches(':')))
        .unwrap_or_else(|| "/etc/localtime".into());
    let data = match fs::read(&path) {
        Ok(d) if d.starts_with(b"TZif") => d,
        _ => return 0,
    };
    let be32 = |b: &[u8]| i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64;
    let counts = |h: &[u8]| -> [usize; 6] {
        let mut c = [0usize; 6];
        for (i, v) in c.iter_mut().enumerate() {
            *v = be32(&h[20 + i * 4..]) as usize;
        }
        c // isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt
    };

    let c1 = counts(&data);
    let v1_len = c1[3] * 5 + c1[4] * 6 + c1[5] + c1[2] * 8 + c1[1] + c1[0];
    let (hdr, tsize) = if data[4] >= b'2' && data.len() > 44 + v1_len + 44 {
        (44 + v1_len, 8)
    } else {
        (0, 4)
    };
    let c = counts(&data[hdr..]);
    let (timecnt, typecnt) = (c[3], c[4]);
    let times = hdr + 44;
    let idxs = times + timecnt * tsize;
    let types = idxs + timecnt;
    if data.len() < types + typecnt * 6 || typecnt == 0 {
        return 0;
    }
    let read_t = |i: usize| -> i64 {
        let p = times + i * tsize;
        if tsize == 8 {
            i64::from_be_bytes(data[p..p + 8].try_into().unwrap())
        } else {
            be32(&data[p..])
        }
    };
    let mut ty = 0usize;
    for i in 0..timecnt {
        if read_t(i) <= now {
            ty = data[idxs + i] as usize;
        } else {
            break;
        }
    }
    be32(&data[types + ty * 6..])
}

fn local_time() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let t = (now + tz_offset(now)).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
}

fn pixels(grid: &[&str; 4]) -> [String; 2] {
    let mut out = [String::new(), String::new()];
    for (line, pair) in out.iter_mut().zip([(0, 1), (2, 3)]) {
        let (top, bot): (Vec<char>, Vec<char>) =
            (grid[pair.0].chars().collect(), grid[pair.1].chars().collect());
        for (t, b) in top.iter().zip(bot.iter()) {
            line.push(match (*t == '#', *b == '#') {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                _ => ' ',
            });
        }
    }
    out
}

fn left_emblem() -> [String; 2] {
    let a = pixels(&[".######.", "########", "########", ".######."]);
    let b = pixels(&["######..", "########", "###..###", "###..###"]);
    let c = pixels(&["######", "######", "###...", "###..."]);
    let stripes = ["▀▀▀▀▀▀".to_string(), "▀▀▀▀▀▀".to_string()];
    [
        format!("{} {} {} {}", a[0], b[0], c[0], stripes[0]),
        format!("{} {} {}™{}", a[1], b[1], c[1], stripes[1]),
    ]
}

fn right_emblem() -> [String; 2] {
    pixels(&["##..##..##", "##.####.##", "##.####.##", "##..##..##"])
}


fn width(s: &str) -> usize {
    s.chars().count()
}

fn justify(words: &[&str], w: usize) -> String {
    let used: usize = words.iter().map(|x| width(x)).sum();
    let gaps = words.len().saturating_sub(1).max(1);
    let free = w.saturating_sub(used).max(gaps);
    let mut s = String::new();
    for (i, word) in words.iter().enumerate() {
        s.push_str(word);
        if i + 1 < words.len() {
            let n = free / gaps + usize::from(i < free % gaps);
            s.push_str(&" ".repeat(n));
        }
    }
    s
}

fn place(line: &mut Vec<char>, col: usize, s: &str) {
    for (i, ch) in s.chars().enumerate() {
        if col + i >= line.len() {
            line.resize(col + i + 1, ' ');
        }
        line[col + i] = ch;
    }
}

fn render(color: &str) -> String {
    let host = hostname();
    let user = username().to_uppercase();
    let layout = format!("({})", keyboard_layout());
    let la: Vec<String> = loadavg().split(" / ").map(str::to_string).collect();
    let run = format!("UP {}  {}", uptime(), local_time());

    let left = left_emblem();
    let right = right_emblem();
    let arrows = ">>>";

    let mid_col = (width(&left[0]).max(width(&left[1])) + 3).max(width(&run) + 2 + 3 + 2);
    let la_min = width("LA") + la.iter().map(|x| width(x)).sum::<usize>() + 2 + 6 * 2;
    let mid_w = width(&host).max(la_min).max(width(&user) + 2 + 3 + 2);
    let right_col = mid_col + mid_w + 3;
    let total = (right_col + width(&right[0])).max(right_col + width(&layout));

    let mut l0 = Vec::new();
    let mut l1 = Vec::new();
    let mut l2 = Vec::new();

    place(&mut l0, 0, &left[0]);
    place(&mut l1, 0, &left[1]);
    place(&mut l0, mid_col, &host);
    let mut words = vec!["LA", la[0].as_str(), "-", la[1].as_str(), "-", la[2].as_str()];
    words.retain(|w| !w.is_empty());
    place(&mut l1, mid_col, &justify(&words, mid_w));
    place(&mut l0, total - width(&right[0]), &right[0]);
    place(&mut l1, total - width(&right[1]), &right[1]);

    place(&mut l2, 0, &run);
    place(&mut l2, mid_col - 3 - 2, arrows);
    place(&mut l2, mid_col, &user);
    place(&mut l2, right_col - 3 - 2, arrows);
    place(&mut l2, total - width(&layout), &layout);

    let lines: Vec<String> = [l0, l1, l2]
        .iter()
        .map(|l| l.iter().collect::<String>().trim_end().to_string())
        .collect();
    format!("{}{}{}", color, lines.join("\n"), reset())
}

const HELP: &str = "\
sysbadge — системная табличка в терминале

Использование: sysbadge [-w] [-c ЦВЕТ]

  -c, --color ЦВЕТ   orange (по умолчанию), blue, red, white или #RRGGBB
  -w, --watch        обновлять каждую секунду (Ctrl+C — выход)
  -h, --help         эта справка

Переменная NO_COLOR=1 отключает цвет.";

fn main() {
    let mut watch = false;
    let mut color_name = String::from("orange");

    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-w" | "--watch" => watch = true,
            "-h" | "--help" => {
                println!("{HELP}");
                return;
            }
            "-c" | "--color" => match args.next() {
                Some(v) => color_name = v,
                None => {
                    eprintln!("sysbadge: после {a} нужен цвет: {COLOR_NAMES}");
                    std::process::exit(2);
                }
            },
            _ if a.starts_with("--color=") => color_name = a["--color=".len()..].to_string(),
            _ => {
                eprintln!("sysbadge: неизвестный аргумент «{a}». Справка: sysbadge --help");
                std::process::exit(2);
            }
        }
    }

    let color = match parse_color(&color_name) {
        Some(c) => color_code(c),
        None => {
            eprintln!("sysbadge: неизвестный цвет «{color_name}». Доступно: {COLOR_NAMES}");
            std::process::exit(2);
        }
    };

    let mut stdout = io::stdout();
    if !watch {
        let _ = writeln!(stdout, "{}", render(&color));
        return;
    }
    loop {
        let _ = write!(stdout, "\x1b[H\x1b[2J{}\n", render(&color));
        let _ = stdout.flush();
        thread::sleep(Duration::from_secs(1));
    }
}
