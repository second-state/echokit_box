use embedded_graphics::{
    framebuffer::{buffer_size, Framebuffer},
    image::GetPixel,
    pixelcolor::{
        raw::{LittleEndian, RawU16},
        Rgb565,
    },
    prelude::*,
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{
        renderer::{CharacterStyle, TextRenderer},
        Alignment, Text,
    },
};
use embedded_text::TextBox;
use u8g2_fonts::U8g2TextStyle;

pub type ColorFormat = Rgb565;

// pub const DEFAULT_BACKGROUND: &[u8] = include_bytes!("../assets/echokit.gif");
pub const DEFAULT_BACKGROUND: &[u8] = include_bytes!("../assets/ht.gif");

use crate::boards::{DISPLAY_HEIGHT, DISPLAY_WIDTH};

pub const LM_PNG: &[u8] = include_bytes!("../assets/lm_320x240.png");
pub const AVATAR_GIF: &[u8] = include_bytes!("../assets/xx.gif");

pub type FlushDisplayFn =
    fn(color_data: &[u8], x_start: i32, y_start: i32, x_end: i32, y_end: i32) -> i32;

pub fn background(gif: &[u8], f: FlushDisplayFn) -> anyhow::Result<()> {
    use image::AnimationDecoder;

    // Create a new framebuffer
    let mut display = Box::new(Framebuffer::<
        ColorFormat,
        _,
        LittleEndian,
        DISPLAY_WIDTH,
        DISPLAY_HEIGHT,
        { buffer_size::<ColorFormat>(DISPLAY_WIDTH, DISPLAY_HEIGHT) },
    >::new());
    display.clear(ColorFormat::WHITE)?;

    let img_gif = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(gif))?;

    for ff in img_gif.into_frames() {
        let frame = ff?;

        let delay = frame.delay();

        let img = frame.into_buffer();

        for (x, y, p) in img.enumerate_pixels() {
            if x >= display.size().width || y >= display.size().height || p[3] == 0 {
                continue;
            }

            display.set_pixel(
                Point {
                    x: x as i32,
                    y: y as i32,
                },
                ColorFormat::new(
                    p[0] / (u8::MAX / ColorFormat::MAX_R),
                    p[1] / (u8::MAX / ColorFormat::MAX_G),
                    p[2] / (u8::MAX / ColorFormat::MAX_B),
                ),
            );
        }

        let now = std::time::Instant::now();
        let delay = std::time::Duration::from(delay);

        f(
            display.data(),
            0,
            0,
            DISPLAY_WIDTH as _,
            DISPLAY_HEIGHT as _,
        );

        let e = now.elapsed();

        log::info!("GIF frame rendered in {:?}", e);

        std::thread::sleep(std::time::Instant::now() - (now + delay));
    }

    Ok(())
}

// TextRenderer + CharacterStyle
#[derive(Debug, Clone)]
pub struct MyTextStyle(pub U8g2TextStyle<ColorFormat>, pub i32);

impl TextRenderer for MyTextStyle {
    type Color = ColorFormat;

    fn draw_string<D>(
        &self,
        text: &str,
        mut position: Point,
        baseline: embedded_graphics::text::Baseline,
        target: &mut D,
    ) -> Result<Point, D::Error>
    where
        D: DrawTarget<Color = Self::Color>,
    {
        position.y += self.1;
        self.0.draw_string(text, position, baseline, target)
    }

    fn draw_whitespace<D>(
        &self,
        width: u32,
        mut position: Point,
        baseline: embedded_graphics::text::Baseline,
        target: &mut D,
    ) -> Result<Point, D::Error>
    where
        D: DrawTarget<Color = Self::Color>,
    {
        position.y += self.1;
        self.0.draw_whitespace(width, position, baseline, target)
    }

    fn measure_string(
        &self,
        text: &str,
        mut position: Point,
        baseline: embedded_graphics::text::Baseline,
    ) -> embedded_graphics::text::renderer::TextMetrics {
        position.y += self.1;
        self.0.measure_string(text, position, baseline)
    }

    fn line_height(&self) -> u32 {
        self.0.line_height()
    }
}

impl CharacterStyle for MyTextStyle {
    type Color = ColorFormat;

    fn set_text_color(&mut self, text_color: Option<Self::Color>) {
        self.0.set_text_color(text_color);
    }

    fn set_background_color(&mut self, background_color: Option<Self::Color>) {
        self.0.set_background_color(background_color);
    }

    fn set_underline_color(
        &mut self,
        underline_color: embedded_graphics::text::DecorationColor<Self::Color>,
    ) {
        self.0.set_underline_color(underline_color);
    }

    fn set_strikethrough_color(
        &mut self,
        strikethrough_color: embedded_graphics::text::DecorationColor<Self::Color>,
    ) {
        self.0.set_strikethrough_color(strikethrough_color);
    }
}

type DisplayTarget = Framebuffer<
    ColorFormat,
    RawU16,
    LittleEndian,
    DISPLAY_WIDTH,
    DISPLAY_HEIGHT,
    { buffer_size::<ColorFormat>(DISPLAY_WIDTH, DISPLAY_HEIGHT) },
>;

pub trait DisplayTargetDrive:
    DrawTarget<Color = ColorFormat> + GetPixel<Color = ColorFormat>
{
    fn new(color: ColorFormat) -> Self;
    fn flush(&mut self) -> anyhow::Result<()>;
    fn fix_background(&mut self) -> anyhow::Result<()>;
}

pub fn display_gif<D: DisplayTargetDrive>(
    display_target: &mut D,
    gif: &[u8],
) -> anyhow::Result<()> {
    use image::AnimationDecoder;
    let img_gif = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(gif))?;

    let mut frames = img_gif.into_frames();
    let mut ff = frames.next();

    loop {
        if ff.is_none() {
            break;
        }

        let frame = ff.unwrap()?;

        let delay = frame.delay();

        let img = frame.into_buffer();
        let pixels = img.enumerate_pixels().map(|(x, y, p)| {
            let (x, y) = if p[3] == 0 {
                (-1, -1)
            } else {
                (x as i32, y as i32)
            };

            Pixel(
                Point { x, y },
                ColorFormat::new(
                    p[0] / (u8::MAX / ColorFormat::MAX_R),
                    p[1] / (u8::MAX / ColorFormat::MAX_G),
                    p[2] / (u8::MAX / ColorFormat::MAX_B),
                ),
            )
        });

        display_target
            .draw_iter(pixels)
            .map_err(|_| anyhow::anyhow!("Failed to draw GIF frame"))?;

        let now = std::time::Instant::now();
        ff = frames.next();
        if ff.is_none() {
            display_target.fix_background()?;
        }

        display_target.flush()?;

        let delay = std::time::Duration::from(delay);

        std::thread::sleep(std::time::Instant::now() - (now + delay));
    }

    Ok(())
}

pub fn display_png<D: DisplayTargetDrive>(
    display_target: &mut D,
    png: &[u8],
    timeout: std::time::Duration,
) -> anyhow::Result<()> {
    let img_reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);

    let img = img_reader.decode().unwrap().to_rgb8();

    let p = img.enumerate_pixels().map(|(x, y, p)| {
        Pixel(
            Point::new(x as i32, y as i32),
            ColorFormat::new(
                p[0] / (u8::MAX / ColorFormat::MAX_R),
                p[1] / (u8::MAX / ColorFormat::MAX_G),
                p[2] / (u8::MAX / ColorFormat::MAX_B),
            ),
        )
    });

    display_target
        .draw_iter(p)
        .map_err(|_| anyhow::anyhow!("Failed to draw PNG image"))?;

    display_target.fix_background()?;

    display_target.flush()?;

    std::thread::sleep(timeout);

    Ok(())
}

pub fn alpha_mix(source: ColorFormat, target: ColorFormat, alpha: f32) -> ColorFormat {
    ColorFormat::new(
        ((1. - alpha) * source.r() as f32 + alpha * target.r() as f32) as u8,
        ((1. - alpha) * source.g() as f32 + alpha * target.g() as f32) as u8,
        ((1. - alpha) * source.b() as f32 + alpha * target.b() as f32) as u8,
    )
}

#[derive(Debug, Clone, Copy)]
pub struct QrPixel(ColorFormat);

impl qrcode::render::Pixel for QrPixel {
    type Image = ((u32, u32), Vec<embedded_graphics::Pixel<ColorFormat>>);

    type Canvas = QrCanvas;

    fn default_color(color: qrcode::Color) -> Self {
        match color {
            qrcode::Color::Dark => QrPixel(ColorFormat::BLACK),
            qrcode::Color::Light => QrPixel(ColorFormat::WHITE),
        }
    }
}

pub struct QrCanvas {
    width: u32,
    height: u32,
    dark_pixel: QrPixel,
    #[allow(unused)]
    light_pixel: QrPixel,
    pixels: Vec<embedded_graphics::Pixel<ColorFormat>>,
}

impl qrcode::render::Canvas for QrCanvas {
    type Pixel = QrPixel;

    type Image = ((u32, u32), Vec<embedded_graphics::Pixel<ColorFormat>>);

    fn new(width: u32, height: u32, dark_pixel: Self::Pixel, light_pixel: Self::Pixel) -> Self {
        Self {
            width,
            height,
            dark_pixel,
            light_pixel,
            pixels: Vec::with_capacity((width * height) as usize),
        }
    }

    fn draw_dark_pixel(&mut self, x: u32, y: u32) {
        if x < self.width && y < self.height {
            self.pixels.push(embedded_graphics::Pixel(
                Point::new(x as i32, y as i32),
                self.dark_pixel.0,
            ));
        }
    }

    fn into_image(self) -> Self::Image {
        ((self.width, self.height), self.pixels)
    }
}

pub struct DisplayArea {
    area: Rectangle,
    text: String,
    render_fn: fn(&DisplayArea, &mut DisplayTarget) -> anyhow::Result<()>,
}

impl DisplayArea {
    pub fn new_text_area(
        area: Rectangle,
        background: Vec<Pixel<ColorFormat>>,
        text: String,
        render_fn: fn(&DisplayArea, &mut DisplayTarget) -> anyhow::Result<()>,
    ) -> Self {
        Self {
            area,
            text,
            render_fn,
        }
    }
}

pub fn get_background_pixels<T: GetPixel<Color = ColorFormat>>(
    display: &T,
    area: Rectangle,
    background_style: PrimitiveStyle<ColorFormat>,
    alpha: f32,
) -> Vec<Pixel<ColorFormat>> {
    area.into_styled(background_style)
        .pixels()
        .map(|p| {
            if let Some(color) = display.pixel(p.0) {
                Pixel(p.0, alpha_mix(color, p.1, alpha))
            } else {
                p
            }
        })
        .collect()
}

pub fn new_display_target() -> Box<DisplayTarget> {
    let mut display_target = Box::new(Framebuffer::<
        ColorFormat,
        _,
        LittleEndian,
        DISPLAY_WIDTH,
        DISPLAY_HEIGHT,
        { buffer_size::<ColorFormat>(DISPLAY_WIDTH, DISPLAY_HEIGHT) },
    >::new());

    display_target.clear(ColorFormat::WHITE).unwrap();

    display_target
}

pub struct ImageArea {
    area: Rectangle,
    pub image_data: Vec<Pixel<ColorFormat>>,
    pub render_fn: fn(&ImageArea, &mut DisplayTarget) -> anyhow::Result<()>,
}

impl ImageArea {
    pub fn new_from_color(area: Rectangle, color: ColorFormat) -> anyhow::Result<Self> {
        let pixels: Vec<Pixel<ColorFormat>> =
            area.points().map(|point| Pixel(point, color)).collect();

        Ok(Self {
            area,
            image_data: pixels,
            render_fn: Self::default_render,
        })
    }

    pub fn new_from_png(area: Rectangle, png_data: &[u8]) -> anyhow::Result<Self> {
        let ht = image::ImageReader::with_format(
            std::io::Cursor::new(png_data),
            image::ImageFormat::Png,
        );
        let img = ht.decode().unwrap().to_rgb8();

        let mut pixels = Vec::with_capacity((area.size.width * area.size.height) as usize);

        for (x, y, p) in img.enumerate_pixels() {
            if x >= area.size.width || y >= area.size.height {
                continue;
            }
            pixels.push(Pixel(
                Point::new(area.top_left.x + x as i32, area.top_left.y + y as i32),
                ColorFormat::new(
                    p[0] / (u8::MAX / ColorFormat::MAX_R),
                    p[1] / (u8::MAX / ColorFormat::MAX_G),
                    p[2] / (u8::MAX / ColorFormat::MAX_B),
                ),
            ));
        }

        Ok(Self {
            area,
            image_data: pixels,
            render_fn: Self::default_render,
        })
    }

    pub fn new_from_qr_code(area: Rectangle, qr_content: &str) -> anyhow::Result<Self> {
        let code = qrcode::QrCode::new(qr_content).unwrap();
        let ((width, height), code_pixel) = code
            .render::<QrPixel>()
            .quiet_zone(true)
            .module_dimensions(4, 4)
            .build();

        let offset_x = if area.size.width > width {
            (area.size.width - width) / 2
        } else {
            0
        };
        let offset_y = if area.size.height > height {
            (area.size.height - height) / 2
        } else {
            0
        };

        let pixels: Vec<Pixel<ColorFormat>> = code_pixel
            .into_iter()
            .map(|p| {
                Pixel(
                    Point::new(
                        p.0.x + area.top_left.x + offset_x as i32,
                        p.0.y + area.top_left.y + offset_y as i32,
                    ),
                    p.1,
                )
            })
            .collect();

        Ok(Self {
            area: Rectangle {
                top_left: area.top_left + Point::new(offset_x as i32, offset_y as i32),
                size: Size::new(width, height),
            },
            image_data: pixels,
            render_fn: Self::default_render,
        })
    }

    pub fn default_render(area: &Self, display: &mut DisplayTarget) -> anyhow::Result<()> {
        display.draw_iter(area.image_data.iter().cloned())?;
        Ok(())
    }
}

pub struct DynamicImage<const N: usize> {
    pub display_index: usize,
    pub image_data: Vec<Vec<Pixel<ColorFormat>>>,
}

impl<const N: usize> DynamicImage<N> {
    pub fn new_from_gif(area: Rectangle, gif_data: &[u8]) -> anyhow::Result<Self> {
        use image::AnimationDecoder;
        let img_gif = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(gif_data))?;

        let frames = img_gif.into_frames();
        let mut image_data: Vec<Vec<Pixel<ColorFormat>>> = Vec::new();
        for ff in frames.take(N) {
            let frame = ff?;

            let img = frame.into_buffer();
            let mut pixels = Vec::with_capacity((area.size.width * area.size.height) as usize);

            for (x, y, p) in img.enumerate_pixels() {
                if x >= area.size.width || y >= area.size.height || p[3] == 0 {
                    continue;
                }
                pixels.push(Pixel(
                    Point::new(area.top_left.x + x as i32, area.top_left.y + y as i32),
                    ColorFormat::new(
                        p[0] / (u8::MAX / ColorFormat::MAX_R),
                        p[1] / (u8::MAX / ColorFormat::MAX_G),
                        p[2] / (u8::MAX / ColorFormat::MAX_B),
                    ),
                ));
            }

            image_data.push(pixels);
        }

        Ok(Self {
            display_index: 0,
            image_data,
        })
    }

    pub fn set_index(&mut self, index: usize) {
        self.display_index = index % N;
    }

    pub fn render<D: DrawTarget<Color = ColorFormat>>(
        &self,
        display: &mut D,
    ) -> Result<(), D::Error> {
        display.draw_iter(self.image_data[self.display_index].iter().cloned())?;
        Ok(())
    }
}

pub struct StartUI {
    pub flush_fn: FlushDisplayFn,
    pub display_target: Box<DisplayTarget>,
}

impl StartUI {
    pub fn new_with_gif(
        mut display_target: Box<DisplayTarget>,
        flush_fn: FlushDisplayFn,
        gif: &[u8],
    ) -> anyhow::Result<Self> {
        use image::AnimationDecoder;
        let img_gif = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(gif)).unwrap();

        let frames = img_gif.into_frames();
        for ff in frames {
            let frame = ff.unwrap();

            let delay = frame.delay();

            let img = frame.into_buffer();
            let pixels = img.enumerate_pixels().map(|(x, y, p)| {
                let (x, y) = if p[3] == 0 {
                    (-1, -1)
                } else {
                    (x as i32, y as i32)
                };

                Pixel(
                    Point { x, y },
                    ColorFormat::new(
                        p[0] / (u8::MAX / ColorFormat::MAX_R),
                        p[1] / (u8::MAX / ColorFormat::MAX_G),
                        p[2] / (u8::MAX / ColorFormat::MAX_B),
                    ),
                )
            });

            display_target.draw_iter(pixels)?;

            let now = std::time::Instant::now();

            flush_fn(
                display_target.data(),
                0,
                0,
                DISPLAY_WIDTH as _,
                DISPLAY_HEIGHT as _,
            );

            let delay = std::time::Duration::from(delay);

            std::thread::sleep(std::time::Instant::now() - (now + delay));
        }

        Ok(Self {
            flush_fn,
            display_target,
        })
    }

    pub fn new_with_png(
        mut display_target: Box<DisplayTarget>,
        flush_fn: FlushDisplayFn,
        png: &[u8],
        delay_ms: u64,
    ) -> anyhow::Result<Self> {
        let ht =
            image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
        let img = ht.decode().unwrap().to_rgb8();

        let p = img.enumerate_pixels().map(|(x, y, p)| {
            Pixel(
                Point::new(x as i32, y as i32),
                ColorFormat::new(
                    p[0] / (u8::MAX / ColorFormat::MAX_R),
                    p[1] / (u8::MAX / ColorFormat::MAX_G),
                    p[2] / (u8::MAX / ColorFormat::MAX_B),
                ),
            )
        });

        p.draw(display_target.as_mut())?;

        flush_fn(
            display_target.data(),
            0,
            0,
            DISPLAY_WIDTH as _,
            DISPLAY_HEIGHT as _,
        );

        std::thread::sleep(std::time::Duration::from_millis(delay_ms));

        Ok(Self {
            flush_fn,
            display_target,
        })
    }
}

pub struct ChatUI {
    state_area: (DisplayArea, bool),
    asr_area: (DisplayArea, bool),
    header_area: (DynamicImage<4>, bool),
    content_area: (DisplayArea, bool),

    pub flush_fn: FlushDisplayFn,
    pub display_target: Box<DisplayTarget>,
}

impl ChatUI {
    pub fn new(
        state_area: DisplayArea,
        asr_area: DisplayArea,
        header_area: DynamicImage<4>,
        content_area: DisplayArea,
        display_target: Box<DisplayTarget>,
        flush_fn: FlushDisplayFn,
    ) -> Self {
        Self {
            state_area: (state_area, true),
            asr_area: (asr_area, true),
            header_area: (header_area, true),
            content_area: (content_area, true),
            flush_fn,
            display_target,
        }
    }

    pub fn set_state(&mut self, state: String) {
        self.state_area.0.text = state;
        self.state_area.1 = true;
    }

    pub fn set_asr(&mut self, asr: String) {
        self.asr_area.0.text = asr;
        self.asr_area.1 = true;
    }

    pub fn set_text(&mut self, content: String) {
        self.content_area.0.text = content;
        self.content_area.1 = true;
    }

    pub fn set_header(&mut self, index: usize) {
        self.header_area.0.set_index(index);
        self.header_area.1 = true;
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        if self.state_area.1 {
            (self.state_area.0.render_fn)(&self.state_area.0, self.display_target.as_mut())?;
            self.state_area.1 = false;
        }

        if self.asr_area.1 {
            (self.asr_area.0.render_fn)(&self.asr_area.0, self.display_target.as_mut())?;
            self.asr_area.1 = false;
        }

        if self.content_area.1 {
            (self.content_area.0.render_fn)(&self.content_area.0, self.display_target.as_mut())?;
            self.content_area.1 = false;
        }

        if self.header_area.1 {
            self.header_area.0.render(self.display_target.as_mut())?;
            self.header_area.1 = false;
        }

        (self.flush_fn)(
            self.display_target.data(),
            0,
            0,
            DISPLAY_WIDTH as _,
            DISPLAY_HEIGHT as _,
        );

        Ok(())
    }
}

pub fn new_chat_ui(start: StartUI) -> anyhow::Result<ChatUI> {
    let StartUI {
        flush_fn,
        display_target,
    } = start;
    let bounding_box = display_target.bounding_box();

    let state_area_box = Rectangle::new(
        bounding_box.top_left + Point::new(96, 0),
        Size::new(bounding_box.size.width - 96, 32),
    );

    let state_area = DisplayArea::new_text_area(
        state_area_box,
        get_background_pixels(
            display_target.as_ref(),
            state_area_box,
            PrimitiveStyleBuilder::new()
                .stroke_color(ColorFormat::CSS_DARK_BLUE)
                .stroke_width(1)
                .fill_color(ColorFormat::CSS_DARK_BLUE)
                .build(),
            0.5,
        ),
        String::new(),
        |area, display| {
            Text::with_alignment(
                &area.text,
                area.area.center(),
                U8g2TextStyle::new(
                    u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                    ColorFormat::CSS_LIGHT_CYAN,
                ),
                Alignment::Center,
            )
            .draw(display)?;
            Ok(())
        },
    );

    let asr_area_box = Rectangle::new(
        bounding_box.top_left + Point::new(96, 32),
        Size::new(bounding_box.size.width - 96, 64),
    );

    let asr_area = DisplayArea::new_text_area(
        asr_area_box,
        get_background_pixels(
            display_target.as_ref(),
            asr_area_box,
            PrimitiveStyleBuilder::new()
                .stroke_color(ColorFormat::CSS_DARK_CYAN)
                .stroke_width(1)
                .fill_color(ColorFormat::CSS_DARK_CYAN)
                .build(),
            0.15,
        ),
        String::new(),
        |area, display| {
            Text::with_alignment(
                &area.text,
                area.area.center(),
                U8g2TextStyle::new(
                    u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                    ColorFormat::CSS_WHEAT,
                ),
                Alignment::Center,
            )
            .draw(display)?;
            Ok(())
        },
    );

    let content_height = bounding_box.size.height - 32 - 64;
    let content_area_box = Rectangle::new(
        bounding_box.top_left + Point::new(0, 32 + 64),
        Size::new(bounding_box.size.width, content_height),
    );

    let content_area = DisplayArea::new_text_area(
        content_area_box,
        get_background_pixels(
            display_target.as_ref(),
            content_area_box,
            PrimitiveStyleBuilder::new()
                .stroke_color(ColorFormat::CSS_BLACK)
                .stroke_width(5)
                .fill_color(ColorFormat::CSS_BLACK)
                .build(),
            0.25,
        ),
        String::new(),
        |area, display| {
            let textbox_style = embedded_text::style::TextBoxStyleBuilder::new()
                .height_mode(embedded_text::style::HeightMode::FitToText)
                .alignment(embedded_text::alignment::HorizontalAlignment::Center)
                .line_height(embedded_graphics::text::LineHeight::Percent(120))
                .paragraph_spacing(16)
                .build();
            let text_box = TextBox::with_textbox_style(
                &area.text,
                area.area,
                MyTextStyle(
                    U8g2TextStyle::new(
                        u8g2_fonts::fonts::u8g2_font_wqy16_t_gb2312,
                        ColorFormat::CSS_WHEAT,
                    ),
                    3,
                ),
                textbox_style,
            );
            text_box.draw(display)?;
            Ok(())
        },
    );

    let header_area_box = Rectangle::new(bounding_box.top_left, Size::new(96, 96));
    let header_area = DynamicImage::new_from_gif(header_area_box, AVATAR_GIF)?;

    Ok(ChatUI::new(
        state_area,
        asr_area,
        header_area,
        content_area,
        display_target,
        flush_fn,
    ))
}

pub struct ConfiguresUI {
    qr_area: ImageArea,
    info_area: DisplayArea,

    pub flush_fn: FlushDisplayFn,
    pub display_target: Box<DisplayTarget>,
}

impl ConfiguresUI {
    pub fn new(
        qr_area: ImageArea,
        info_area: DisplayArea,
        display_target: Box<DisplayTarget>,
        flush_fn: FlushDisplayFn,
    ) -> Self {
        Self {
            qr_area,
            info_area,
            flush_fn,
            display_target,
        }
    }

    pub fn set_info(&mut self, info: String) {
        self.info_area.text = info;
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        (self.info_area.render_fn)(&self.info_area, self.display_target.as_mut())?;
        (self.qr_area.render_fn)(&self.qr_area, self.display_target.as_mut())?;

        (self.flush_fn)(
            self.display_target.data(),
            0,
            0,
            DISPLAY_WIDTH as _,
            DISPLAY_HEIGHT as _,
        );

        Ok(())
    }
}

pub fn new_config_ui(start: StartUI, qr_content: &str) -> anyhow::Result<ConfiguresUI> {
    let StartUI {
        flush_fn,
        display_target,
    } = start;
    let bounding_box = display_target.bounding_box();

    let height = bounding_box.size.height;

    let qr_area_box = Rectangle::new(
        bounding_box.top_left + Point::new(0, height as i32 / 3),
        Size::new(bounding_box.size.width, 2 * height / 3),
    );
    let qr_area = ImageArea::new_from_qr_code(qr_area_box, qr_content)?;

    let info_area = DisplayArea::new_text_area(
        bounding_box,
        get_background_pixels(
            display_target.as_ref(),
            bounding_box,
            PrimitiveStyleBuilder::new()
                .stroke_color(ColorFormat::CSS_DARK_BLUE)
                .stroke_width(1)
                .fill_color(ColorFormat::CSS_DARK_BLUE)
                .build(),
            0.25,
        ),
        String::new(),
        |area, display| {
            Text::with_alignment(
                &area.text,
                area.area.top_left + Point::new(area.area.size.width as i32 / 2, 32),
                U8g2TextStyle::new(
                    u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                    ColorFormat::CSS_WHEAT,
                ),
                Alignment::Center,
            )
            .draw(display)?;
            Ok(())
        },
    );

    Ok(ConfiguresUI::new(
        qr_area,
        info_area,
        display_target,
        flush_fn,
    ))
}
