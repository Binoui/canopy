//! The made-up sites the README's screenshots browse. Nothing here is fetched from the web.

use gtk::gdk_pixbuf::{prelude::*, PixbufLoader};

pub struct Answer {
    pub kind: &'static str,
    pub header: &'static str,
    pub body: Vec<u8>,
}

struct Site {
    host: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: &'static str,
    style: &'static str,
}

const SITES: &[Site] = &[
    Site {
        host: "bonsaiwiki.example",
        name: "Bonsai Wiki",
        glyph: "W",
        color: "#3d6b4f",
        style: WIKI,
    },
    Site {
        host: "rootbound.example",
        name: "Rootbound",
        glyph: "R",
        color: "#c2552d",
        style: FORUM,
    },
    Site {
        host: "mossandwire.example",
        name: "Moss & Wire",
        glyph: "M",
        color: "#7a8f3c",
        style: BLOG,
    },
    Site {
        host: "koen-nursery.example",
        name: "Kōen Nursery",
        glyph: "K",
        color: "#2b6cb0",
        style: SHOP,
    },
    Site {
        host: "slowrail.example",
        name: "Slowrail",
        glyph: "S",
        color: "#b83280",
        style: SHOP,
    },
    Site {
        host: "templegardens.example",
        name: "Temple Gardens",
        glyph: "T",
        color: "#b7791f",
        style: BLOG,
    },
    Site {
        host: "ferrous.example",
        name: "The Ferrous Book",
        glyph: "F",
        color: "#dd6b20",
        style: WIKI,
    },
    Site {
        host: "kilnlog.example",
        name: "Kiln Log",
        glyph: "K",
        color: "#805ad5",
        style: BLOG,
    },
    Site {
        host: "weather.example",
        name: "Skywatch",
        glyph: "S",
        color: "#3182ce",
        style: SHOP,
    },
    Site {
        host: "mail.example",
        name: "Postbox",
        glyph: "P",
        color: "#e53e3e",
        style: SHOP,
    },
    Site {
        host: "paperlane.example",
        name: "Paper Lane",
        glyph: "P",
        color: "#319795",
        style: BLOG,
    },
    Site {
        host: "tinytracker.example",
        name: "Tiny Tracker",
        glyph: "T",
        color: "#4a5568",
        style: FORUM,
    },
];

const BASE: &str = "*{box-sizing:border-box}body{margin:0;font:17px/1.6 Inter,system-ui,sans-serif;color:#1f2421}a{color:#1d6b46}img,svg{max-width:100%}header.bar{display:flex;align-items:center;gap:12px;padding:14px 32px;font-weight:650}header.bar .mark{display:grid;place-items:center;width:28px;height:28px;border-radius:7px;color:#fff;font-size:15px}header.bar nav{margin-left:auto;display:flex;gap:22px;font-weight:450;font-size:15px;opacity:.75}main{max-width:820px;margin:0 auto;padding:36px 32px 80px}h1{font-size:38px;line-height:1.15;margin:0 0 10px}h2{font-size:22px;margin:34px 0 8px}.lede{font-size:20px;color:#4b554f;margin:0 0 22px}.meta{font-size:14px;color:#6b756f;margin-bottom:22px}.hero{border-radius:14px;overflow:hidden;margin:0 0 26px}.box{border:1px solid #dfe5e0;border-radius:12px;padding:16px 18px;margin:16px 0;background:#fff}.file{display:flex;align-items:center;gap:12px;text-decoration:none;border:1px solid #d6dccf;border-radius:10px;padding:12px 14px;margin:10px 0;background:#fff;color:inherit}.file b{display:block}.file small{color:#6b756f}.file i{font-style:normal;margin-left:auto;font-size:14px;color:#1d6b46;font-weight:600}";
const WIKI: &str = "body{background:#fbfbf8}header.bar{background:#fff;border-bottom:1px solid #e3e6df}h1,h2{font-family:Georgia,'Noto Serif',serif;font-weight:500}h1{border-bottom:1px solid #d9ddd4;padding-bottom:10px}.infobox{float:right;width:250px;margin:0 0 18px 26px;border:1px solid #d9ddd4;border-radius:10px;background:#fff;font-size:14px;overflow:hidden}.infobox dl{margin:0;padding:12px 14px;display:grid;grid-template-columns:auto 1fr;gap:4px 12px}.infobox dt{color:#6b756f}.infobox dd{margin:0}.infobox dt::after{content:' '}";
const FORUM: &str = "body{background:#f3f1ec}header.bar{background:#26211c;color:#fff}h1{font-size:30px}.post{background:#fff;border:1px solid #e2ddd3;border-radius:12px;padding:16px 20px;margin:14px 0}.post .who{display:flex;align-items:center;gap:10px;font-size:14px;color:#6b665e;margin-bottom:6px}.post .who b{color:#26211c}.post .who span.av{width:26px;height:26px;border-radius:50%;display:inline-block}.post p{margin:6px 0}";
const BLOG: &str = "body{background:#f7f3ea}header.bar{background:transparent;border-bottom:1px solid #e4dccb}h1,h2{font-family:Georgia,'Noto Serif',serif}main{max-width:740px}";
const SHOP: &str = "body{background:#fff}header.bar{border-bottom:1px solid #e6e9ee}.buy{display:grid;grid-template-columns:1fr 1fr;gap:32px;align-items:start}.price{font-size:30px;font-weight:700;margin:6px 0 14px}.btn{display:inline-block;background:#1f2421;color:#fff;border-radius:10px;padding:11px 20px;font-weight:600;text-decoration:none}.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:16px}.grid .box{margin:0}";

fn site(host: &str) -> Option<&'static Site> {
    SITES.iter().find(|site| site.host == host)
}

/// A soft drawing that stands in for a photograph: hills, a pot and a tree in the site's color.
fn picture(color: &str, sky: &str, height: u32) -> String {
    format!(
        "<svg class='hero' viewBox='0 0 820 {height}' xmlns='http://www.w3.org/2000/svg' preserveAspectRatio='xMidYMid slice'><rect width='820' height='{height}' fill='{sky}'/><circle cx='660' cy='70' r='38' fill='#fff' opacity='.55'/><path d='M0 {a} C160 {b} 300 {c} 470 {a} S720 {b} 820 {c} V{height} H0Z' fill='{color}' opacity='.28'/><path d='M0 {d} C200 {a} 380 {d} 560 {e} S760 {a} 820 {d} V{height} H0Z' fill='{color}' opacity='.45'/><g transform='translate(330 {t})'><path d='M40 150h100l-12 46H52z' fill='#3a2f28'/><rect x='30' y='140' width='120' height='14' rx='5' fill='#4b3d33'/><path d='M88 142c-4-34 10-48-6-74s-44-20-52-44' fill='none' stroke='#4b3d33' stroke-width='9' stroke-linecap='round'/><ellipse cx='24' cy='22' rx='46' ry='22' fill='{color}'/><ellipse cx='98' cy='54' rx='54' ry='24' fill='{color}' opacity='.9'/><ellipse cx='60' cy='-6' rx='38' ry='18' fill='{color}' opacity='.8'/></g></svg>",
        a = height - 70,
        b = height - 130,
        c = height - 90,
        d = height - 40,
        e = height - 76,
        t = height as i32 - 215,
    )
}

fn page(site: &Site, title: &str, body: &str) -> Answer {
    let html = format!(
        "<!doctype html><meta charset='utf-8'><link rel='icon' href='/favicon.png'><title>{title} · {name}</title><style>{BASE}{style}</style><header class='bar'><span class='mark' style='background:{color}'>{glyph}</span>{name}<nav><span>Browse</span><span>Recent</span><span>About</span></nav></header><main>{body}</main>",
        name = site.name,
        style = site.style,
        color = site.color,
        glyph = site.glyph,
    );
    Answer {
        kind: "text/html; charset=utf-8",
        header: "",
        body: html.into_bytes(),
    }
}

fn favicon(site: &Site) -> Option<Answer> {
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='64' height='64' rx='15' fill='{}'/><text x='32' y='45' font-family='Inter,sans-serif' font-weight='700' font-size='38' text-anchor='middle' fill='#fff'>{}</text></svg>",
        site.color, site.glyph
    );
    let loader = PixbufLoader::new();
    loader.write(svg.as_bytes()).ok()?;
    loader.close().ok()?;
    Some(Answer {
        kind: "image/png",
        header: "",
        body: loader.pixbuf()?.save_to_bufferv("png", &[]).ok()?,
    })
}

fn file(name: &'static str, size: usize) -> Answer {
    let header = match name {
        "calendar" => "Content-Disposition: attachment; filename=\"repotting-calendar.pdf\"\r\n",
        "mix" => "Content-Disposition: attachment; filename=\"soil-mix.csv\"\r\n",
        _ => "Content-Disposition: attachment; filename=\"juniper-before-after.zip\"\r\n",
    };
    Answer {
        kind: "application/octet-stream",
        header,
        body: vec![b'.'; size],
    }
}

pub fn answer(host: &str, path: &str) -> Option<Answer> {
    let site = site(host)?;
    if path == "/favicon.png" || path == "/favicon.ico" {
        return favicon(site);
    }
    let green = picture("#3d6b4f", "#dfe9dc", 300);
    Some(match (host, path) {
        ("bonsaiwiki.example", "/wiki/Repotting") => page(site, "Repotting", &format!(
            "<h1>Repotting</h1><p class='lede'>Repotting renews the soil of a bonsai and keeps its roots compact enough to live in a shallow pot.</p><div class='infobox'>{}<dl><dt>Season</dt><dd>Late winter to early spring</dd><dt>Interval</dt><dd>2 to 5 years</dd><dt>Tools</dt><dd>Root hook, shears, mesh, wire</dd></dl></div><p>A tree is repotted when its roots circle the pot and water stops draining freely. Most trees are repotted as the buds begin to swell, before the first leaves open. Growers disagree about how late is too late: see the long thread on <a id='thread' href='http://rootbound.example/t/4821'>when to repot junipers</a>.</p><h2>Soil</h2><p>The classic mix is equal parts <a id='akadama' href='/wiki/Akadama'>akadama</a>, pumice and lava rock. Conifers such as <a id='juniper' href='/wiki/Juniper'>juniper</a> prefer a mix with less akadama, because they resent wet feet.</p><h2>Method</h2><p>The old soil is combed out with a <a id='hook' href='/wiki/Root_pruning'>root hook</a>, long roots are shortened, and the tree is tied into fresh, free-draining soil.</p><h2>Aftercare</h2><p>Keep the tree out of wind and strong sun for three weeks, water when the top of the soil dries, and do not feed until new growth hardens.</p>",
            picture("#3d6b4f", "#e6ede2", 170)
        )),
        ("bonsaiwiki.example", "/wiki/Akadama") => page(site, "Akadama", &format!(
            "<h1>Akadama</h1><p class='lede'>Akadama is a granular, clay-like volcanic soil that holds water and nutrients while staying open to air.</p><div class='infobox'>{}<dl><dt>Origin</dt><dd>Kantō plain, Japan</dd><dt>Grain</dt><dd>1 to 6 mm</dd><dt>Lasts</dt><dd>2 to 3 years</dd></dl></div><p>Fresh akadama is hard and tan. As roots grow through it, it breaks down into finer particles, which is why trees in akadama are repotted every few years. Hard grades last longer and are worth the price for conifers. A good source for the 14 litre bags is <a id='shop' href='http://koen-nursery.example/soil/akadama-14l'>Kōen Nursery</a>.</p><h2>Sifting</h2><p>Sift out the dust before use. Dust fills the gaps between grains and stops the pot from draining.</p>",
            picture("#b7791f", "#f3e7cf", 170)
        )),
        ("bonsaiwiki.example", "/wiki/Juniper") => page(site, "Juniper", &format!(
            "<h1>Juniper</h1><p class='lede'>Junipers are the most widely grown conifers in bonsai, valued for their deadwood and dense foliage pads.</p><div class='infobox'>{}<dl><dt>Genus</dt><dd><i>Juniperus</i></dd><dt>Light</dt><dd>Full sun</dd><dt>Repot</dt><dd>Every 3 to 4 years</dd></dl></div><p>They are kept outdoors all year. Foliage is thinned with scissors rather than pinched. Roots are reduced gently: a juniper keeps part of its old soil at every repotting.</p>",
            picture("#2f7a55", "#dcebe2", 170)
        )),
        ("bonsaiwiki.example", "/wiki/Root_pruning") => page(site, "Root pruning", &format!(
            "<h1>Root pruning</h1><p class='lede'>Root pruning shortens thick roots so that fine feeder roots grow close to the trunk.</p>{green}<p>Remove roots that grow straight down, cut back the thickest by a third, and leave the fine roots near the surface.</p>"
        )),
        ("rootbound.example", "/t/4821") => page(site, "When do you repot junipers?", &thread()),
        ("mossandwire.example", "/2026/spring-repotting-log") => page(site, "My spring repotting log", &format!(
            "<h1>My spring repotting log</h1><div class='meta'>12 March 2026 · 6 minute read</div>{}<p class='lede'>Eleven trees, two weekends, one cracked pot. Here is what I did this year and what I would change.</p><p>I repotted the junipers last, in the first week of April, when the tips had turned bright green. They kept a third of their old soil. The maples went first and got the hard root work.</p><h2>Files</h2><a class='file' id='calendar' href='/files/calendar'><span><b>Repotting calendar</b><small>PDF · one page per species</small></span><i>Download</i></a><a class='file' id='mix' href='/files/mix'><span><b>Soil mix by species</b><small>CSV · ratios I used this year</small></span><i>Download</i></a><a class='file' id='photos' href='/files/photos'><span><b>Juniper before and after</b><small>ZIP · 14 photographs</small></span><i>Download</i></a><h2>What I would change</h2><p>Sift the akadama the day before. Wire the pots before the tree is out of the ground. And read <a id='wrong' href='/2025/what-i-got-wrong'>last year's list of mistakes</a> before starting, not after.</p>",
            picture("#7a8f3c", "#efe6cf", 300)
        )),
        ("mossandwire.example", "/2025/what-i-got-wrong") => page(site, "What I got wrong last year", &format!(
            "<h1>What I got wrong last year</h1><div class='meta'>2 November 2025 · 4 minute read</div>{}<p class='lede'>Three trees sulked all summer. All three were my fault.</p><p>I bare-rooted a juniper, fed too early, and left a freshly repotted elm in the wind.</p>",
            picture("#a0522d", "#f1e2d0", 300)
        )),
        ("mossandwire.example", "/files/calendar") => file("calendar", 184_320),
        ("mossandwire.example", "/files/mix") => file("mix", 3_412),
        ("mossandwire.example", "/files/photos") => file("photos", 2_411_724),
        ("koen-nursery.example", "/soil/akadama-14l") => page(site, "Akadama 14 L, hard grade", &format!(
            "<div class='buy'>{}<div><h1>Akadama 14 L, hard grade</h1><div class='price'>€ 24.50</div><p>Double-fired, sifted to 3 to 6 mm. Ships in two days.</p><a class='btn'>Add to basket</a><div class='box'><b>Works well with</b><br>Pumice 10 L · Lava rock 10 L · <a id='pots' href='/pots/unglazed'>Unglazed pots</a></div></div></div>",
            picture("#b7791f", "#f5ead6", 420)
        )),
        ("koen-nursery.example", "/pots/unglazed") => page(site, "Unglazed pots", &format!(
            "<h1>Unglazed pots</h1><p class='lede'>Brown, grey and red clay for conifers. Made in small batches.</p><div class='grid'><div class='box'>{}<b>Oval, 31 cm</b><br>€ 58</div><div class='box'>{}<b>Rectangle, 27 cm</b><br>€ 49</div><div class='box'>{}<b>Round, 18 cm</b><br>€ 32</div></div>",
            picture("#8b5e3c", "#efe3d3", 520), picture("#6b6f76", "#e6e8ea", 520), picture("#a2452f", "#f2ddd3", 520)
        )),
        ("slowrail.example", "/routes/tokyo-kyoto") => page(site, "Tokyo to Kyoto by rail", &format!(
            "<h1>Tokyo to Kyoto by rail</h1><p class='lede'>Two hours and a quarter on the fast line, or a full day on local trains along the coast.</p>{}<div class='grid'><div class='box'><b>Fast</b><br>2 h 15 · from ¥14,170</div><div class='box'><b>Coastal</b><br>8 h 40 · ¥8,360</div><div class='box'><b>Night bus</b><br>7 h 30 · from ¥4,200</div></div><p>Staying a while? <a id='gardens' href='http://templegardens.example/kyoto/november'>Kyoto's gardens in November</a> are worth the detour.</p>",
            picture("#b83280", "#f6e3ee", 280)
        )),
        ("templegardens.example", "/kyoto/november") => page(site, "Kyoto's gardens in November", &format!(
            "<h1>Kyoto's gardens in November</h1><div class='meta'>Updated for autumn 2026</div>{}<p class='lede'>The maples turn from the north of the city to the south over about three weeks.</p><p>Start early at <a id='daitoku' href='/kyoto/daitoku-ji'>Daitoku-ji</a>, where the sub-temple gardens open at nine and stay quiet until eleven. The <a id='omiya' href='/kyoto/omiya-bonsai'>Ōmiya bonsai collection</a> is a short walk from there.</p>",
            picture("#c05621", "#f7e6d3", 300)
        )),
        ("templegardens.example", "/kyoto/daitoku-ji") => page(site, "Daitoku-ji", &format!(
            "<h1>Daitoku-ji</h1>{}<p class='lede'>A walled temple complex of more than twenty sub-temples, four of them open all year.</p><p>Kōtō-in's maple path is the one in every photograph. Come on a weekday.</p>",
            picture("#b7791f", "#f3ead9", 300)
        )),
        ("templegardens.example", "/kyoto/omiya-bonsai") => page(site, "Ōmiya bonsai collection", &format!(
            "<h1>Ōmiya bonsai collection</h1>{green}<p class='lede'>Sixty trees on open benches, some of them older than the temple wall behind them.</p>"
        )),
        ("ferrous.example", "/book/iterators") => page(site, "Iterators", "<h1>Iterators</h1><p class='lede'>An iterator yields one item at a time and does no work until it is asked.</p><div class='box'><code>let total: u32 = rings.iter().map(|r| r.width).sum();</code></div><p>Adapters such as <a id='closures' href='/book/closures'>map and filter</a> take closures and return new iterators.</p>"),
        ("ferrous.example", "/book/closures") => page(site, "Closures", "<h1>Closures</h1><p class='lede'>A closure is a function that can capture values from where it is written.</p><div class='box'><code>let thicker = |ring: &amp;Ring| ring.width &gt; limit;</code></div>"),
        ("kilnlog.example", "/glazes/ash") => page(site, "Ash glazes at cone 10", &format!(
            "<h1>Ash glazes at cone 10</h1><div class='meta'>Firing 41</div>{}<p class='lede'>Wood ash, feldspar and a little clay. Runny, green and never the same twice.</p>",
            picture("#805ad5", "#e9e3f7", 300)
        )),
        (_, "/") => page(site, "Home", &format!("<h1>{}</h1>{green}<p class='lede'>Welcome back.</p>", site.name)),
        _ => return None,
    })
}

fn thread() -> String {
    let post = |color: &str, who: &str, when: &str, text: &str| {
        format!("<div class='post'><div class='who'><span class='av' style='background:{color}'></span><b>{who}</b> · {when}</div>{text}</div>")
    };
    format!(
        "<h1>When do you repot junipers?</h1><div class='meta'>Conifers · 38 replies</div>{}{}{}{}",
        post("#c2552d", "hinoki_dan", "3 days ago", "<p>Mine are pushing new tips already and I have not touched the roots. Is early April too late in zone 8?</p>"),
        post("#3d6b4f", "old_shimpaku", "3 days ago", "<p>Not too late. I wait for the tips to turn bright green, then go. Never bare-root: keep a wedge of old soil. The wiki page on <a id='pruning' href='http://bonsaiwiki.example/wiki/Root_pruning'>root pruning</a> has the cuts drawn out.</p>"),
        post("#2b6cb0", "wren", "2 days ago", "<p>Seconding that. I wrote up my whole spring in <a id='log' href='http://mossandwire.example/2026/spring-repotting-log'>a repotting log</a>, with the calendar I follow, and <a id='wrong' href='http://mossandwire.example/2025/what-i-got-wrong'>what I got wrong</a> the year before.</p>"),
        post("#805ad5", "kiln_and_moss", "yesterday", "<p>If you need a pot for it, the <a id='pots' href='http://koen-nursery.example/pots/unglazed'>unglazed ones from Kōen</a> drain well and suit a juniper.</p>"),
    )
}
