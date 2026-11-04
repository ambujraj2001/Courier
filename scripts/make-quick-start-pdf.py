"""Generates docs/Courier-Quick-Start.pdf, the one-page quick start.

Run on macOS (uses the system Arial and Courier New fonts):
    python3 scripts/make-quick-start-pdf.py . docs/Courier-Quick-Start.pdf
Requires: pip install reportlab
"""

import sys
from reportlab.lib.colors import HexColor, white
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen import canvas
from reportlab.platypus import Paragraph
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.lib.fonts import addMapping

FONTS = "/System/Library/Fonts/Supplemental/"
pdfmetrics.registerFont(TTFont("Body", FONTS + "Arial.ttf"))
pdfmetrics.registerFont(TTFont("Body-Bold", FONTS + "Arial Bold.ttf"))
pdfmetrics.registerFont(TTFont("Mono", FONTS + "Courier New.ttf"))
addMapping("Body", 0, 0, "Body")
addMapping("Body", 1, 0, "Body-Bold")
addMapping("Body", 0, 1, "Body")
addMapping("Body", 1, 1, "Body-Bold")

ROOT = sys.argv[1]
OUT = sys.argv[2]

INK = HexColor("#111317")
INK2 = HexColor("#1b1f25")
TEAL = HexColor("#0d9488")
TEAL_L = HexColor("#2dd4bf")
TEXT = HexColor("#111827")
MUTED = HexColor("#5b6472")
FAINT = HexColor("#9aa2ae")
BORDER = HexColor("#e3e6ea")
SOFT = HexColor("#f3f6f6")
CODE_TXT = HexColor("#e6edf3")
CODE_DIM = HexColor("#7d8590")

W, H = A4
M = 36  # page margin

c = canvas.Canvas(OUT, pagesize=A4)
c.setTitle("Courier — Quick start")
c.setAuthor("Courier")
c.setSubject("Install and try Courier on macOS, Windows and Linux")
c.setFont("Body", 9)


def para(text, x, y_top, width, size=9, color=TEXT, font="Body", leading=None, align=0):
    style = ParagraphStyle("p", fontName=font, fontSize=size, leading=leading or size * 1.35, textColor=color, alignment=align)
    p = Paragraph(text, style)
    _, h = p.wrap(width, 1000)
    p.drawOn(c, x, y_top - h)
    return h


def section_title(num, title, x, y):
    c.setFillColor(TEAL)
    c.circle(x + 8, y + 3.5, 8, stroke=0, fill=1)
    c.setFillColor(white)
    c.setFont("Body-Bold", 9)
    c.drawCentredString(x + 8, y, num)
    c.setFillColor(TEXT)
    c.setFont("Body-Bold", 13)
    c.drawString(x + 22, y - 0.5, title)


def code_block(x, y_top, width, label, hint, lines):
    """Dark rounded block with a label row and monospace command lines."""
    pad = 10
    line_h = 10.2
    h = pad + 13 + len(lines) * line_h + pad - 2
    c.setFillColor(INK)
    c.roundRect(x, y_top - h, width, h, 7, stroke=0, fill=1)
    # Label row.
    c.setFillColor(TEAL_L)
    c.setFont("Body-Bold", 9)
    c.drawString(x + pad, y_top - pad - 7, label)
    c.setFillColor(CODE_DIM)
    c.setFont("Body", 7.5)
    c.drawRightString(x + width - pad, y_top - pad - 7, hint)
    # Code.
    y = y_top - pad - 13 - 8
    # No "$" prompt: commands must copy and paste cleanly.
    for line in lines:
        c.setFont("Mono", 7.3)
        c.setFillColor(CODE_TXT)
        c.drawString(x + pad, y, line)
        y -= line_h
    return h


# ------------------------------------------------------------------ header
HEADER_H = 104
c.setFillColor(INK)
c.rect(0, H - HEADER_H, W, HEADER_H, stroke=0, fill=1)
# Subtle accent strip.
c.setFillColor(TEAL)
c.rect(0, H - HEADER_H, W, 3, stroke=0, fill=1)
icon = ImageReader(f"{ROOT}/assets/icon/icon_256x256.png")
c.drawImage(icon, M - 6, H - 92, 70, 70, mask="auto")
c.setFillColor(white)
c.setFont("Body-Bold", 30)
c.drawString(M + 72, H - 52, "Courier")
c.setFillColor(TEAL_L)
c.setFont("Body-Bold", 11)
c.drawString(M + 74, H - 70, "QUICK START  ·  TRY IT IN TWO MINUTES")
c.setFillColor(HexColor("#aeb6c2"))
c.setFont("Body", 9.5)
c.drawString(M + 74, H - 86, "A fast, local-first API client. No account, no cloud — your requests stay on your machine.")
# Version pill.
pill = "v0.2.0   ·   macOS   ·   Windows   ·   Linux"
c.setFont("Body-Bold", 8)
pw = c.stringWidth(pill, "Body-Bold", 8) + 22
c.setFillColor(INK2)
c.setStrokeColor(HexColor("#31363f"))
c.roundRect(W - M - pw, H - 50, pw, 20, 10, stroke=1, fill=1)
c.setFillColor(HexColor("#d7dce3"))
c.drawCentredString(W - M - pw / 2, H - 43, pill)

# ------------------------------------------------------------------ intro row
top = H - HEADER_H - 18
shot_w = 268
shot_h = shot_w * 860 / 1360
shot_x = W - M - shot_w
shot = ImageReader(f"{ROOT}/docs/screenshots/request-light.png")
# Shadow + frame.
c.setFillColor(HexColor("#d9dde3"))
c.roundRect(shot_x + 2, top - shot_h - 3, shot_w, shot_h, 6, stroke=0, fill=1)
c.drawImage(shot, shot_x, top - shot_h, shot_w, shot_h)
c.setStrokeColor(BORDER)
c.setLineWidth(0.8)
c.roundRect(shot_x, top - shot_h, shot_w, shot_h, 3, stroke=1, fill=0)

left_w = shot_x - M - 18
c.setFillColor(TEXT)
c.setFont("Body-Bold", 13)
c.drawString(M, top - 10, "What you can do")
features = [
    ("Send any HTTP request", "with params, headers, JSON, forms, files or GraphQL."),
    ("Every common auth", "Bearer, Basic, API key, OAuth 1 & 2, JWT, AWS, Digest."),
    ("Variables & environments", "switch between dev, staging and prod in one click."),
    ("Collections & history", "organise requests; everything you send is remembered."),
    ("Scripts & tests", "JavaScript pre-request and test scripts with pm.*"),
    ("Run whole collections", "with data files, then export JUnit reports."),
    ("Postman & cURL", "import and export Collection v2.1 and cURL commands."),
]
y = top - 30
for title, rest in features:
    c.setFillColor(TEAL)
    c.circle(M + 3, y - 3.3, 2.4, stroke=0, fill=1)
    h = para(f"<b>{title}</b> <font color='#5b6472'>— {rest}</font>", M + 12, y, left_w - 12, size=8.6, leading=11)
    y -= h + 4

# ------------------------------------------------------------------ install
y = min(y, top - shot_h) - 18
section_title("1", "Install — one command", M, y)
c.setFillColor(MUTED)
c.setFont("Body", 8.5)
c.drawRightString(W - M, y, "Each command downloads the latest release, installs it and opens Courier.")
y -= 14
full_w = W - 2 * M
gap = 8
h = code_block(
    M,
    y,
    full_w,
    "Ubuntu / Debian",
    "paste into Terminal  ·  asks for your password once",
    [
        "wget -O /tmp/courier.deb https://github.com/ambujraj2001/Courier/releases/latest/download/courier_amd64.deb &&",
        "sudo apt install -y /tmp/courier.deb && courier",
    ],
)
y -= h + gap
h = code_block(
    M,
    y,
    full_w,
    "Windows 10 / 11",
    "paste into PowerShell  ·  puts Courier.exe on your Desktop",
    [
        '$f = "$env:USERPROFILE\\Desktop\\Courier.exe"',
        "iwr https://github.com/ambujraj2001/Courier/releases/latest/download/Courier-windows-x86_64.exe -OutFile $f",
        "Start-Process $f",
    ],
)
y -= h + gap
h = code_block(
    M,
    y,
    full_w,
    "macOS (Apple Silicon)",
    "paste into Terminal  ·  Intel Mac: change arm64 to x86_64 (twice)",
    [
        "cd /tmp && curl -fLO https://github.com/ambujraj2001/Courier/releases/latest/download/Courier-macos-arm64.dmg &&",
        "hdiutil attach -nobrowse -quiet -mountpoint /tmp/courier-dmg Courier-macos-arm64.dmg &&",
        "cp -R /tmp/courier-dmg/Courier.app /Applications && hdiutil detach -quiet /tmp/courier-dmg &&",
        "open /Applications/Courier.app",
    ],
)
y -= h + 16

# ------------------------------------------------------------------ try it + shortcuts
col_gap = 22
col_w = (full_w - col_gap) / 2
section_title("2", "Try it", M, y)
section_title("3", "Handy shortcuts", M + col_w + col_gap, y)
y -= 18

# Steps.
steps = [
    ("Open a new request", "Press <b>Cmd+T</b> on Mac or <b>Ctrl+T</b> on Windows/Linux."),
    ("Paste this into the URL bar", None),
    ("Send it", "Press <b>Cmd+Enter</b> / <b>Ctrl+Enter</b>. You should see <font color='#16a34a'><b>201 Created</b></font> and a JSON reply."),
]
sy = y
for i, (title, body) in enumerate(steps, start=1):
    c.setFillColor(SOFT)
    c.setStrokeColor(BORDER)
    c.circle(M + 9, sy - 6, 9, stroke=1, fill=1)
    c.setFillColor(TEAL)
    c.setFont("Body-Bold", 9)
    c.drawCentredString(M + 9, sy - 9.2, str(i))
    c.setFillColor(TEXT)
    c.setFont("Body-Bold", 9.5)
    c.drawString(M + 24, sy - 9.5, title)
    sy -= 14
    if body:
        h = para(body, M + 24, sy, col_w - 24, size=8.5, color=MUTED, leading=11)
        sy -= h + 7
    else:
        sample = [
            "curl https://jsonplaceholder.typicode.com/posts \\",
            "  -H 'Content-Type: application/json' \\",
            "  -d '{\"title\":\"Hello from Courier\",\"userId\":1}'",
        ]
        bh = 8 + len(sample) * 9.4 + 4
        c.setFillColor(INK)
        c.roundRect(M + 24, sy - bh, col_w - 24, bh, 5, stroke=0, fill=1)
        c.setFont("Mono", 6.9)
        c.setFillColor(CODE_TXT)
        ly = sy - 10
        for line in sample:
            c.drawString(M + 31, ly, line)
            ly -= 9.4
        sy -= bh + 7
para(
    "<b>Bonus:</b> press <b>Cmd+O</b> / <b>Ctrl+O</b> to import a Postman collection, environment or any cURL command.",
    M,
    sy - 2,
    col_w,
    size=8.3,
    color=MUTED,
    leading=11,
)

# Shortcuts table.
sx = M + col_w + col_gap
rows = [
    ("New request", "Cmd+T", "Ctrl+T"),
    ("Send request", "Cmd+Enter", "Ctrl+Enter"),
    ("Save", "Cmd+S", "Ctrl+S"),
    ("Import", "Cmd+O", "Ctrl+O"),
    ("Focus the URL bar", "Cmd+L", "Ctrl+L"),
    ("Search the sidebar", "Cmd+F", "Ctrl+F"),
    ("Toggle console", "Opt+Cmd+C", "Ctrl+Alt+C"),
    ("All shortcuts", "Cmd+/", "Ctrl+/"),
]
row_h = 17
table_h = row_h * (len(rows) + 1)
c.setStrokeColor(BORDER)
c.setFillColor(white)
c.roundRect(sx, y - table_h, col_w, table_h, 6, stroke=1, fill=1)
c.setFillColor(SOFT)
c.roundRect(sx, y - row_h, col_w, row_h, 6, stroke=0, fill=1)
c.rect(sx, y - row_h, col_w, row_h / 2, stroke=0, fill=1)
c.setFont("Body-Bold", 7.2)
c.setFillColor(FAINT)
cx = [sx + 10, sx + col_w * 0.45, sx + col_w * 0.69]
for x, label in zip(cx, ["ACTION", "MAC", "WIN / LINUX"]):
    c.drawString(x, y - 11.5, label)
ty = y - row_h
for i, (action, mac, other) in enumerate(rows):
    c.setStrokeColor(BORDER)
    c.line(sx, ty, sx + col_w, ty)
    c.setFillColor(TEXT)
    c.setFont("Body", 8.4)
    c.drawString(cx[0], ty - 11.8, action)
    for x, key in ((cx[1], mac), (cx[2], other)):
        c.setFont("Body-Bold", 7.4)
        kw = c.stringWidth(key, "Body-Bold", 7.4) + 8
        c.setFillColor(SOFT)
        c.setStrokeColor(BORDER)
        c.roundRect(x - 2, ty - 14, kw, 12, 3, stroke=1, fill=1)
        c.setFillColor(TEXT)
        c.drawString(x + 2, ty - 11.2, key)
    ty -= row_h

# ------------------------------------------------------------------ footer
fy = 34
c.setStrokeColor(BORDER)
c.line(M, fy + 26, W - M, fy + 26)
c.setFillColor(TEXT)
c.setFont("Body-Bold", 8.5)
c.drawString(M, fy + 12, "github.com/ambujraj2001/Courier")
c.linkURL("https://github.com/ambujraj2001/Courier", (M, fy + 8, M + 150, fy + 22), relative=0)
c.setFillColor(MUTED)
c.setFont("Body", 8)
c.drawString(M, fy, "Source code, issues and downloads.")
right = [
    "Your data stays on your computer:",
    "~/Library/Application Support/Courier  ·  ~/.local/share/Courier  ·  %APPDATA%\\Courier",
]
c.setFillColor(TEXT)
c.setFont("Body-Bold", 8)
c.drawRightString(W - M, fy + 12, right[0])
c.setFillColor(MUTED)
c.setFont("Body", 7.4)
c.drawRightString(W - M, fy, right[1])
c.setFillColor(FAINT)
c.setFont("Body", 6.8)
c.drawCentredString(W / 2, 16, "Windows may show “Windows protected your PC” the first time: click More info, then Run anyway.")

c.showPage()
c.save()
print("wrote", OUT)
