use embedded_graphics::{
    framebuffer::{buffer_size, Framebuffer},
    image::GetPixel,
    pixelcolor::{
        raw::{LittleEndian, RawU16},
        Rgb565,
    },
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{
        renderer::{CharacterStyle, TextRenderer},
        Alignment, Text,
    },
};
use embedded_text::TextBox;
use u8g2_fonts::U8g2TextStyle;

pub type ColorFormat = Rgb565;

pub const DEFAULT_BACKGROUND: &[u8] = include_bytes!("../assets/echokit.gif");

use crate::boards::{DISPLAY_HEIGHT, DISPLAY_WIDTH};

pub type FlushDisplayFn =
    fn(color_data: &[u8], x_start: i32, y_start: i32, x_end: i32, y_end: i32) -> i32;

pub fn backgroud(gif: &[u8], f: FlushDisplayFn) -> Result<(), std::convert::Infallible> {
    let image = tinygif::Gif::<ColorFormat>::from_slice(gif).unwrap();

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

    for frame in image.frames() {
        if !frame.is_transparent {
            display.clear(ColorFormat::WHITE)?;
        }
        frame.draw(display.as_mut())?;
        f(
            display.data(),
            0,
            0,
            DISPLAY_WIDTH as _,
            DISPLAY_HEIGHT as _,
        );
        let delay_ms = frame.delay_centis * 10;
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
    }

    Ok(())
}

const ALPHA: f32 = 0.5;

// TextRenderer + CharacterStyle
#[derive(Debug, Clone)]
struct MyTextStyle(U8g2TextStyle<ColorFormat>, i32);

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

pub struct UI {
    pub state: String,
    state_area: Rectangle,
    state_background: Vec<Pixel<ColorFormat>>,
    pub text: String,
    text_area: Rectangle,
    text_background: Vec<Pixel<ColorFormat>>,

    display: Box<
        Framebuffer<
            ColorFormat,
            RawU16,
            LittleEndian,
            DISPLAY_WIDTH,
            DISPLAY_HEIGHT,
            { buffer_size::<ColorFormat>(DISPLAY_WIDTH, DISPLAY_HEIGHT) },
        >,
    >,

    flush_fn: FlushDisplayFn,
}

const COLOR_WIDTH: u32 = 2;

fn alpha_mix(source: ColorFormat, target: ColorFormat, alpha: f32) -> ColorFormat {
    ColorFormat::new(
        ((1. - alpha) * source.r() as f32 + alpha * target.r() as f32) as u8,
        ((1. - alpha) * source.g() as f32 + alpha * target.g() as f32) as u8,
        ((1. - alpha) * source.b() as f32 + alpha * target.b() as f32) as u8,
    )
}

fn flush_area<const COLOR_WIDTH: u32>(
    data: &[u8],
    size: Size,
    area: Rectangle,
    flash_fn: FlushDisplayFn,
) -> i32 {
    let start_y = area.top_left.y as u32;
    let end_y = start_y + area.size.height;

    let start_index = start_y * size.width * COLOR_WIDTH;
    let data_len = area.size.height * size.width * COLOR_WIDTH;
    if let Some(area_data) = data.get(start_index as usize..(start_index + data_len) as usize) {
        flash_fn(
            area_data,
            0,
            start_y as i32,
            size.width as i32,
            end_y as i32,
        )
    } else {
        log::warn!("flush_area error: data out of bounds");
        log::warn!(
            "start_index: {start_index}, area_len: {data_len}, data_len: {}",
            data.len()
        );
        -1
    }
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

impl UI {
    pub fn new(backgroud_gif: Option<&[u8]>, flush_fn: FlushDisplayFn) -> anyhow::Result<Self> {
        let mut display = Box::new(Framebuffer::<
            ColorFormat,
            _,
            LittleEndian,
            DISPLAY_WIDTH,
            DISPLAY_HEIGHT,
            { buffer_size::<ColorFormat>(DISPLAY_WIDTH, DISPLAY_HEIGHT) },
        >::new());

        display.clear(ColorFormat::WHITE).unwrap();

        let state_area = Rectangle::new(
            display.bounding_box().top_left + Point::new(0, 0),
            Size::new(DISPLAY_WIDTH as u32, 32),
        );
        let text_area = Rectangle::new(
            display.bounding_box().top_left + Point::new(0, 32),
            Size::new(DISPLAY_WIDTH as u32, DISPLAY_HEIGHT as u32 - 32),
        );

        if let Some(gif) = backgroud_gif {
            let image = tinygif::Gif::<ColorFormat>::from_slice(gif)
                .map_err(|e| anyhow::anyhow!("Failed to parse GIF: {:?}", e))?;
            for frame in image.frames() {
                frame.draw(display.as_mut()).unwrap();
            }
        }

        let img = display.as_image();

        let state_pixels: Vec<Pixel<ColorFormat>> = state_area
            .into_styled(
                PrimitiveStyleBuilder::new()
                    .stroke_color(ColorFormat::CSS_DARK_BLUE)
                    .stroke_width(1)
                    .fill_color(ColorFormat::CSS_DARK_BLUE)
                    .build(),
            )
            .pixels()
            .map(|p| {
                if let Some(color) = img.pixel(p.0) {
                    Pixel(p.0, alpha_mix(color, p.1, ALPHA))
                } else {
                    p
                }
            })
            .collect();

        let box_pixels: Vec<Pixel<ColorFormat>> = text_area
            .into_styled(
                PrimitiveStyleBuilder::new()
                    .stroke_color(ColorFormat::CSS_BLACK)
                    .stroke_width(5)
                    .fill_color(ColorFormat::CSS_BLACK)
                    .build(),
            )
            .pixels()
            .map(|p| {
                if let Some(color) = img.pixel(p.0) {
                    Pixel(p.0, alpha_mix(color, p.1, ALPHA))
                } else {
                    p
                }
            })
            .collect();

        Ok(Self {
            state: String::new(),
            state_background: state_pixels,
            text: String::new(),
            text_background: box_pixels,
            display,
            state_area,
            text_area,
            flush_fn,
        })
    }

    pub fn display_flush(&mut self) -> anyhow::Result<()> {
        self.state_background
            .iter()
            .cloned()
            .draw(self.display.as_mut())?;
        self.text_background
            .iter()
            .cloned()
            .draw(self.display.as_mut())?;

        Text::with_alignment(
            &self.state,
            self.state_area.center(),
            U8g2TextStyle::new(
                u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                ColorFormat::CSS_LIGHT_CYAN,
            ),
            Alignment::Center,
        )
        .draw(self.display.as_mut())?;

        let textbox_style = embedded_text::style::TextBoxStyleBuilder::new()
            .height_mode(embedded_text::style::HeightMode::FitToText)
            .alignment(embedded_text::alignment::HorizontalAlignment::Center)
            .line_height(embedded_graphics::text::LineHeight::Percent(120))
            .paragraph_spacing(16)
            .build();
        let text_box = TextBox::with_textbox_style(
            &self.text,
            self.text_area,
            MyTextStyle(
                U8g2TextStyle::new(
                    u8g2_fonts::fonts::u8g2_font_wqy16_t_gb2312,
                    ColorFormat::CSS_WHEAT,
                ),
                3,
            ),
            textbox_style,
        );
        text_box.draw(self.display.as_mut())?;

        for i in 0..5 {
            let e = flush_area::<COLOR_WIDTH>(
                self.display.data(),
                self.display.size(),
                Rectangle::new(
                    self.state_area.top_left,
                    Size::new(
                        self.text_area.size.width,
                        self.text_area.size.height + self.state_area.size.height,
                    ),
                ),
                self.flush_fn,
            );
            if e == 0 {
                break;
            }
            log::warn!("flush_display error: {} retry {i}", e);
        }
        Ok(())
    }

    pub fn display_qrcode(&mut self, qr_context: &str) -> anyhow::Result<()> {
        let code = qrcode::QrCode::new(qr_context).unwrap();
        let ((width, height), code_pixel) = code
            .render::<QrPixel>()
            .quiet_zone(true)
            .module_dimensions(4, 4)
            .build();

        self.state_background
            .iter()
            .cloned()
            .draw(self.display.as_mut())?;
        self.text_background
            .iter()
            .cloned()
            .draw(self.display.as_mut())?;

        self.display
            .cropped(&Rectangle::new(
                self.text_area.top_left
                    + Point::new(
                        ((self.text_area.size.width - width) / 2) as i32,
                        (self.text_area.size.height - height) as i32,
                    ),
                Size::new(width, height),
            ))
            .draw_iter(code_pixel)?;

        Text::with_alignment(
            &self.state,
            self.state_area.center(),
            U8g2TextStyle::new(
                u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                ColorFormat::CSS_LIGHT_CYAN,
            ),
            Alignment::Center,
        )
        .draw(self.display.as_mut())?;

        let textbox_style = embedded_text::style::TextBoxStyleBuilder::new()
            .height_mode(embedded_text::style::HeightMode::FitToText)
            .alignment(embedded_text::alignment::HorizontalAlignment::Center)
            .line_height(embedded_graphics::text::LineHeight::Percent(120))
            .paragraph_spacing(12)
            .build();
        let text_box = TextBox::with_textbox_style(
            &self.text,
            self.text_area,
            MyTextStyle(
                U8g2TextStyle::new(
                    u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                    ColorFormat::CSS_WHEAT,
                ),
                3,
            ),
            textbox_style,
        );
        text_box.draw(self.display.as_mut())?;

        for i in 0..5 {
            let e = flush_area::<COLOR_WIDTH>(
                self.display.data(),
                self.display.size(),
                Rectangle::new(
                    self.state_area.top_left,
                    Size::new(
                        self.text_area.size.width,
                        self.text_area.size.height + self.state_area.size.height,
                    ),
                ),
                self.flush_fn,
            );
            if e == 0 {
                break;
            }
            log::warn!("flush_display error: {} retry {i}", e);
        }
        Ok(())
    }
}
