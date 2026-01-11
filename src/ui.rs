use embedded_graphics::{
    image::GetPixel,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::renderer::{CharacterStyle, TextRenderer},
};
use u8g2_fonts::U8g2TextStyle;

pub type ColorFormat = Rgb565;

// pub const DEFAULT_BACKGROUND: &[u8] = include_bytes!("../assets/echokit.gif");
pub const DEFAULT_BACKGROUND: &[u8] = include_bytes!("../assets/ht.gif");

pub const LM_PNG: &[u8] = include_bytes!("../assets/lm_320x240.png");
pub const AVATAR_GIF: &[u8] = include_bytes!("../assets/avatar.gif");

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

pub struct ImageArea {
    pub image_data: Vec<Pixel<ColorFormat>>,
}

impl ImageArea {
    pub fn new_from_color(area: Rectangle, color: ColorFormat) -> anyhow::Result<Self> {
        let pixels: Vec<Pixel<ColorFormat>> =
            area.points().map(|point| Pixel(point, color)).collect();

        Ok(Self { image_data: pixels })
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

        Ok(Self { image_data: pixels })
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
            // area: Rectangle {
            //     top_left: area.top_left + Point::new(offset_x as i32, offset_y as i32),
            //     size: Size::new(width, height),
            // },
            image_data: pixels,
        })
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
        let new_idx = index % N;
        if new_idx == self.display_index {
            self.display_index = 0;
        } else {
            self.display_index = index % N;
        }
    }

    pub fn render<D: DrawTarget<Color = ColorFormat>>(
        &self,
        display: &mut D,
    ) -> Result<(), D::Error> {
        display.draw_iter(self.image_data[self.display_index].iter().cloned())?;
        Ok(())
    }
}
