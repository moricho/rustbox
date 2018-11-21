extern crate libc;

use std::mem;
use std::fs::{OpenOptions, File};
use std::io::Write;
use std::os::unix::io::AsRawFd;

use libc::termios;
use libc::c_int;

macro_rules! build_term_code {
    ($name:ident, $code:expr) => {
        pub struct $name;

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, concat!("\x1B[", $code))
            }
        }

    }
}

mod termcodes {
    build_term_code!(EnterCa, "?1049h\x1b[22;0;0t");
    build_term_code!(ExitCa, "?1049l\x1b[23;0;0t");
    build_term_code!(ClearScreen, "H\x1b[2J");
    build_term_code!(HideCursor, "?25l");
    build_term_code!(ShowCursor, "?25h");
    build_term_code!(SGR0, "m\x0f");

    pub struct EnterKeypad;
    impl std::fmt::Display for EnterKeypad {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "\x1B=")
        }
    }
    pub struct ExitKeypad;
    impl std::fmt::Display for ExitKeypad {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "\x1B>")
        }
    }
}


/// Buffered file writing
///
/// Mostly adapted from std::io::BufWriter
struct BufferedFile {
    inner: File,
    buf: Vec<u8>,
}

impl BufferedFile {
    pub fn new(f: File) -> BufferedFile {
        BufferedFile {
            inner: f,
            buf: Vec::new(),
        }
    }

    fn flush_inner(&mut self) -> std::io::Result<()> {
        let mut written = 0;
        let len = self.buf.len();
        let mut ret = Ok(());

        while written < len {
            match self.inner.write(&self.buf[written..]) {
                Ok(0) => {
                    ret = Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "failed to write data"));
                    break;
                }
                Ok(n) => written += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => { ret = Err(e); break }
            }
        }
        if written > 0 {
            self.buf.drain(..written);
        }

        ret
    }
}

impl Write for BufferedFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Write::write(&mut self.buf, buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_inner().and_then(|()| self.inner.flush())
    }
}


impl Drop for BufferedFile {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}


pub fn get_terminal_attr() -> termios {
    extern "C" {
        pub fn tcgetattr(fd: c_int, termptr: *const termios) -> c_int;
    }
    unsafe {
        let mut ios = mem::zeroed();
        tcgetattr(0, &mut ios);
        ios
    }
}


pub fn set_terminal_attr(t: &termios) -> i32 {
    extern "C" {
        pub fn tcsetattr(fd: c_int, opt: c_int, termptr: *const termios) -> c_int;
    }
    unsafe { tcsetattr(0, 0, t) }
}

#[derive(Copy, Clone)]
pub enum Style {
    Normal,
    Underline,
    Bold,
    Blink,
    Reverse,
}

#[derive(Copy, Clone)]
pub enum Color {
    Black,
    Red,
    White,
}

impl Color {
    pub fn as_256_color(&self) -> u16 {
        match self {
            Color::Black => 0,
            Color::Red => 1,
            Color::White => 7,
        }
    }
}

#[derive(Copy, Clone)]
pub struct Cell {
    ch: char,
    bg: Color,
    fg: Color,
    style: Style,
}



pub struct RustBox {
    orig_ios: termios,
    outf: BufferedFile,


    // TODO(gchp): do we need two buffers?
    front_buffer: Vec<Vec<Cell>>,
    back_buffer: Vec<Vec<Cell>>,

    width: u16,
    height: u16,
}

impl RustBox {
    pub fn new() -> RustBox {
        let orig_ios = get_terminal_attr();
        let mut ios = get_terminal_attr();

        ios.c_iflag &= !(libc::IGNBRK | libc::BRKINT | libc:: PARMRK | libc::ISTRIP
                         | libc::INLCR | libc::IGNCR | libc::ICRNL | libc::IXON);
        ios.c_oflag &= !libc::OPOST;
        ios.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
        ios.c_cflag &= !(libc::CSIZE | libc::PARENB);
        ios.c_cflag |= libc::CS8;
        ios.c_cc[libc::VMIN] = 0;
        ios.c_cc[libc::VTIME] = 0;

        let outf = OpenOptions::new().read(true).write(true).open("/dev/tty").unwrap();
        // TODO(gchp): find out what this is about. See termbox tb_init.
        unsafe { libc::tcsetattr(outf.as_raw_fd(), libc::TCSAFLUSH, &ios); }

        let win_size = libc::winsize { ws_col: 0, ws_row: 0, ws_xpixel: 0, ws_ypixel: 0};
        unsafe { libc::ioctl(outf.as_raw_fd(), libc::TIOCGWINSZ, &win_size); }

        let mut buffered_file = BufferedFile::new(outf);

        set_terminal_attr(&ios);


        write!(buffered_file, "{}", termcodes::EnterCa);
        write!(buffered_file, "{}", termcodes::EnterKeypad);
        write!(buffered_file, "{}", termcodes::HideCursor);
        write!(buffered_file, "{}", termcodes::SGR0);
        write!(buffered_file, "{}", termcodes::ClearScreen);

        let _ = buffered_file.flush();



        let mut back_buffer = Vec::new();
        for _i in 0..win_size.ws_row {
            let mut row = Vec::new();
            for _j in 0..win_size.ws_col {
                row.push(Cell { ch: 'x', fg: Color::White, bg: Color::Black, style: Style::Normal })
            }
            back_buffer.push(row);
        }

        RustBox {
            orig_ios: orig_ios,
            outf: buffered_file,

            front_buffer: back_buffer.clone(),
            back_buffer: back_buffer,
            width: win_size.ws_col,
            height: win_size.ws_row,
        }
    }

    pub fn print_char(&mut self, x: usize, y: usize, style: Style, fg: Color, bg: Color, ch: char) {
        let cell = &mut self.back_buffer[y][x];

        cell.ch = ch;
        cell.bg = bg;
        cell.fg = fg;
        cell.style = style;
    }

    pub fn present(&mut self) {
        // TODO(gchp): do we need multiple buffers here?
        self.front_buffer = self.back_buffer.clone();

        for (i, _row) in self.front_buffer.iter().enumerate() {
            for cell in &self.front_buffer[i] {
                // reset
                write!(self.outf, "{}", termcodes::SGR0);

                match cell.style {
                    Style::Normal => {}
                    Style::Underline => { write!(self.outf, "\x1b[4m"); }
                    Style::Bold => { write!(self.outf, "\x1b[1m"); }
                    Style::Blink => { write!(self.outf, "\x1b[5m"); }
                    Style::Reverse => { write!(self.outf, "\x1b[7m"); }
                }

                // TODO(gchp): this currently assumes 256 colors
                let fg = cell.fg.as_256_color() & 0xFF;
                let bg = cell.bg.as_256_color() & 0xFF;

                write!(self.outf, "\x1b[38;5;{}m", fg);
                write!(self.outf, "\x1b[48;5;{}m", bg);

                write!(self.outf, "{}", cell.ch);

                // reset fg
                // write!(self.outf, "\x1b[39m");

                // reset bg
                // write!(self.outf, "\x1b[49m");
            }
        }

        let _ = self.outf.flush();

    }
}


impl Drop for RustBox {
    fn drop(&mut self) {
        write!(self.outf, "{}", termcodes::ShowCursor);
        write!(self.outf, "{}", termcodes::ClearScreen);
        write!(self.outf, "{}", termcodes::ExitCa);
        write!(self.outf, "{}", termcodes::ExitKeypad);

        set_terminal_attr(&self.orig_ios);
    }
}
