#[cfg(feature = "box")]
pub mod atom_box;
#[cfg(feature = "box")]
pub use atom_box::*;

#[cfg(all(feature = "boards", not(feature = "_no_default")))]
pub mod base;
#[cfg(all(feature = "boards", not(feature = "_no_default")))]
pub use base::*;

#[cfg(feature = "cube")]
pub mod cube;
#[cfg(feature = "cube")]
pub use cube::*;

#[cfg(feature = "cube2")]
pub mod cube2;
#[cfg(feature = "cube2")]
pub use cube2::*;

#[cfg(feature = "i2c")]
pub type I2CInitFn = fn(&mut esp_idf_svc::hal::i2c::I2cDriver<'static>) -> anyhow::Result<()>;
#[cfg(feature = "i2c")]
pub type I2CLoopFn = fn(
    &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    &crate::audio::EventTx,
) -> anyhow::Result<()>;

#[cfg(feature = "i2c")]
pub fn init_i2c<
    I2C: esp_idf_svc::hal::i2c::I2c,
    P: esp_idf_svc::hal::peripheral::Peripheral<P = I2C> + 'static,
>(
    config: esp_idf_svc::hal::i2c::config::Config,
    i2c: P,
    sda: esp_idf_svc::hal::gpio::AnyIOPin,
    scl: esp_idf_svc::hal::gpio::AnyIOPin,
    event_tx: crate::audio::EventTx,
    tasks: Vec<(I2CInitFn, I2CLoopFn)>,
    stack_size: usize,
    loop_timeout_ms: u32,
) -> anyhow::Result<()> {
    if tasks.is_empty() {
        log::warn!("No I2C tasks to run");
        return Ok(());
    }

    let i2c_driver = esp_idf_svc::hal::i2c::I2cDriver::new(i2c, sda, scl, &config)
        .map_err(|e| anyhow::anyhow!("Failed to create I2C driver: {:?}", e))?;

    _ = std::thread::Builder::new()
        .stack_size(stack_size)
        .spawn(move || {
            log::info!(
                "Starting I2C worker thread in core {:?}",
                esp_idf_svc::hal::cpu::core()
            );
            let mut i2c_driver = i2c_driver;
            for (init_fn, _) in &tasks {
                if let Err(e) = init_fn(&mut i2c_driver) {
                    log::error!("I2C init function error: {:?}", e);
                }
            }
            loop {
                let now = std::time::Instant::now();
                for (_, loop_fn) in &tasks {
                    if let Err(e) = loop_fn(&mut i2c_driver, &event_tx) {
                        log::error!("I2C loop function error: {:?}", e);
                    }
                }
                let elapsed = now.elapsed();
                if elapsed.as_millis() < loop_timeout_ms as u128 {
                    std::thread::sleep(std::time::Duration::from_millis(
                        loop_timeout_ms as u64 - elapsed.as_millis() as u64,
                    ));
                }
            }
        });

    Ok(())
}

#[cfg(feature = "mfrc522")]
fn decode_ndef_in_mifare_ultralight<D: crate::peripheral::mfrc522::MfrcDriver>(
    mfrc522: &mut crate::peripheral::mfrc522::MFRC522<D>,
    timeout: esp_idf_svc::hal::delay::TickType_t,
) -> Result<Vec<String>, crate::peripheral::mfrc522::consts::PCDErrorCode> {
    let mut buff = [0; 18];

    let mut ndef_buffer = vec![];

    for page in (0..16).step_by(4) {
        let mut bytes_count = 18;
        mfrc522.mifare_read(page, &mut buff, &mut bytes_count, timeout)?;
        ndef_buffer.extend_from_slice(&buff[..16]);
    }

    let n = ndef_buffer[22] as usize;

    let messages = ndef::Message::try_from(&ndef_buffer[23..23 + n]).map_err(|e| {
        log::error!("Error parsing NDEF message: {:?}", e);
        crate::peripheral::mfrc522::consts::PCDErrorCode::Error
    })?;

    let mut r = vec![];
    for record in messages.records {
        if let ndef::Payload::RTD(ndef::RecordType::Text { txt, .. }) = record.payload {
            r.push(txt);
        }
    }

    Ok(r)
}

#[cfg(feature = "mfrc522")]
pub fn init_mfrc522(i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>) -> anyhow::Result<()> {
    let d = crate::peripheral::mfrc522::drivers::I2CDriver::new(i2c, 0x28);
    let mut mfrc522 = crate::peripheral::mfrc522::MFRC522::new(d);
    if let Err(e) = mfrc522.pcd_init(esp_idf_svc::hal::delay::TickType::new_millis(1000).0) {
        log::error!("Error initializing MFRC522: {:?}", e);
        return Err(anyhow::anyhow!("Error initializing MFRC522: {:?}", e));
    }

    if mfrc522.pcd_is_init(esp_idf_svc::hal::delay::TickType::new_millis(1000).0) {
        log::info!("MFRC522 initialized successfully");
        Ok(())
    } else {
        log::error!("Error checking MFRC522 initialization");
        Err(anyhow::anyhow!("Error checking MFRC522 initialization"))
    }
}

#[cfg(feature = "mfrc522")]
pub fn mfrc522_loop(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    evt_tx: &crate::audio::EventTx,
) -> anyhow::Result<()> {
    use crate::peripheral::mfrc522::consts::PICCType;

    let timeout = esp_idf_svc::hal::delay::TickType::new_millis(1000).0;

    let d = crate::peripheral::mfrc522::drivers::I2CDriver::new(i2c, 0x28);
    let mut mfrc522 = crate::peripheral::mfrc522::MFRC522::new(d);

    match mfrc522.picc_is_new_card_present(timeout) {
        Ok(_) => {
            match mfrc522.get_card(crate::peripheral::mfrc522::consts::UidSize::Four, timeout) {
                Ok(card) => {
                    log::info!("Card UID: {}", card.get_number());
                    let picc_type = PICCType::from_sak(card.sak);

                    log::info!("PICC Type: {:?}", picc_type);

                    if !matches!(picc_type, PICCType::PiccTypeMifareUL) {
                        return Ok(());
                    }

                    match decode_ndef_in_mifare_ultralight(&mut mfrc522, timeout) {
                        Ok(texts) => {
                            for text in texts {
                                log::info!("NDEF Text Record: {}", text);
                                evt_tx
                                    .blocking_send(crate::app::Event::ServerUrl(text))
                                    .unwrap_or_else(|e| {
                                        log::error!("Failed to send ServerUrl event: {:?}", e);
                                    });
                            }
                        }
                        Err(e) => {
                            log::error!("Error decoding NDEF message: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Error getting card UID: {:?}", e);
                }
            }
            _ = mfrc522.picc_halta(timeout);
        }
        Err(crate::peripheral::mfrc522::consts::PCDErrorCode::Timeout) => {
            // No card present
        }
        Err(e) => {
            log::error!("Error checking for new card: {:?}", e);
        }
    }

    Ok(())
}

#[allow(unused)]
pub fn backlight_init(
    bl_pin: esp_idf_svc::hal::gpio::AnyIOPin,
) -> anyhow::Result<esp_idf_svc::hal::ledc::LedcDriver<'static>> {
    use esp_idf_svc::hal;
    let config = hal::ledc::config::TimerConfig::new()
        .resolution(hal::ledc::Resolution::Bits13)
        .frequency(hal::units::Hertz(5000));
    let time = unsafe { hal::ledc::TIMER0::new() };
    let timer_driver = hal::ledc::LedcTimerDriver::new(time, &config)?;

    let ledc_driver =
        hal::ledc::LedcDriver::new(unsafe { hal::ledc::CHANNEL0::new() }, timer_driver, bl_pin)?;

    Ok(ledc_driver)
}

const LEDC_MAX_DUTY: u32 = (1 << 13) - 1;
#[allow(unused)]
pub fn set_backlight<'d>(
    ledc_driver: &mut esp_idf_svc::hal::ledc::LedcDriver<'d>,
    light: u8,
) -> anyhow::Result<()> {
    let light = 100.min(light) as u32;
    let duty = LEDC_MAX_DUTY - (81 * (100 - light));
    let duty = if light == 0 { 0 } else { duty };
    ledc_driver.set_duty(duty)?;
    Ok(())
}

#[cfg(not(feature = "custom_ui"))]
pub mod ui {
    use super::*;

    use embedded_graphics::{
        framebuffer::{buffer_size, Framebuffer},
        image::GetPixel,
        pixelcolor::raw::{LittleEndian, RawU16},
        prelude::*,
        primitives::{PrimitiveStyleBuilder, Rectangle},
        text::{Alignment, Text},
        Drawable,
    };
    use u8g2_fonts::U8g2TextStyle;

    use crate::ui::{ColorFormat, DisplayTargetDrive, DynamicImage, ImageArea};

    pub type DisplayBuffer = FrameBuffer;

    type Framebuffer_ = Framebuffer<
        ColorFormat,
        RawU16,
        LittleEndian,
        DISPLAY_WIDTH,
        DISPLAY_HEIGHT,
        { buffer_size::<ColorFormat>(DISPLAY_WIDTH, DISPLAY_HEIGHT) },
    >;

    struct PixelsTarget<'a> {
        pixels: &'a mut Vec<Pixel<ColorFormat>>,
        bounding_box: Rectangle,
    }

    impl Dimensions for PixelsTarget<'_> {
        fn bounding_box(&self) -> Rectangle {
            self.bounding_box
        }
    }

    impl DrawTarget for PixelsTarget<'_> {
        type Color = ColorFormat;
        type Error = core::convert::Infallible;

        fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = embedded_graphics::Pixel<Self::Color>>,
        {
            self.pixels.extend(pixels);
            Ok(())
        }
    }

    pub struct FrameBuffer {
        buffers: Box<Framebuffer_>,
        background_buffers: Box<Framebuffer_>,
    }

    impl Dimensions for FrameBuffer {
        fn bounding_box(&self) -> Rectangle {
            Rectangle::new(
                Point::new(0, 0),
                Size::new(DISPLAY_WIDTH as u32, DISPLAY_HEIGHT as u32),
            )
        }
    }

    impl DrawTarget for FrameBuffer {
        type Color = ColorFormat;
        type Error = core::convert::Infallible;

        fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = embedded_graphics::Pixel<Self::Color>>,
        {
            self.buffers.draw_iter(pixels)?;
            Ok(())
        }
    }

    impl GetPixel for FrameBuffer {
        type Color = ColorFormat;

        fn pixel(&self, point: Point) -> Option<Self::Color> {
            self.buffers.pixel(point)
        }
    }

    impl DisplayTargetDrive for FrameBuffer {
        fn new(color: ColorFormat) -> Self {
            let mut s = Self {
                buffers: Box::new(Framebuffer::new()),
                background_buffers: Box::new(Framebuffer::new()),
            };

            s.buffers.clear(color).unwrap();
            s.background_buffers.clear(color).unwrap();

            s
        }
        fn flush(&mut self) -> anyhow::Result<()> {
            let bounding_box = self.bounding_box();
            let x_start = bounding_box.top_left.x as i32;
            let y_start = bounding_box.top_left.y as i32;
            let x_end = bounding_box.top_left.x + bounding_box.size.width as i32;
            let y_end = bounding_box.top_left.y + bounding_box.size.height as i32;

            let e = flush_display(self.buffers.data(), x_start, y_start, x_end, y_end);
            if e != 0 {
                return Err(anyhow::anyhow!("Failed to flush display: error code {}", e));
            }

            self.buffers.clone_from(&self.background_buffers);

            Ok(())
        }

        fn fix_background(&mut self) -> anyhow::Result<()> {
            self.background_buffers.clone_from(&self.buffers);
            Ok(())
        }
    }

    const AVATAR_SIZE: u32 = 96;
    pub struct ChatUI<const N: usize> {
        state_text: String,
        state_text_pixels: Vec<Pixel<ColorFormat>>,

        asr_text: String,
        asr_text_pixels: Vec<Pixel<ColorFormat>>,

        content: String,
        content_pixels: Vec<Pixel<ColorFormat>>,

        avatar: DynamicImage<N>,
    }

    impl<const N: usize> ChatUI<N> {
        pub fn new(avatar: DynamicImage<N>) -> Self {
            Self {
                state_text: String::new(),
                state_text_pixels: Vec::with_capacity(DISPLAY_WIDTH * 32),
                asr_text: String::new(),
                asr_text_pixels: Vec::with_capacity(DISPLAY_WIDTH * 32),
                content: String::new(),
                content_pixels: Vec::with_capacity(DISPLAY_WIDTH * DISPLAY_HEIGHT / 4),
                avatar: avatar,
            }
        }

        pub fn set_state(&mut self, text: String) {
            if self.state_text != text {
                self.state_text = text;
                self.state_text_pixels.clear();
            }
        }

        pub fn set_asr(&mut self, text: String) {
            if self.asr_text != text {
                self.asr_text = text;
                self.asr_text_pixels.clear();
            }
        }

        pub fn set_text(&mut self, text: String) {
            if self.content != text {
                self.content = text;
                self.content_pixels.clear();
            }
        }

        pub fn set_avatar_index(&mut self, index: usize) {
            self.avatar.set_index(index);
        }

        pub fn render_to_target(&mut self, target: &mut FrameBuffer) -> anyhow::Result<()> {
            let bounding_box = target.bounding_box();

            self.avatar.render(target)?;

            let (state_area_box, asr_area_box, content_area_box) = Self::layout(bounding_box);

            if self.state_text_pixels.is_empty() {
                let mut pixel_target = PixelsTarget {
                    pixels: &mut self.state_text_pixels,
                    bounding_box,
                };
                Text::with_alignment(
                    &self.state_text,
                    state_area_box.center(),
                    U8g2TextStyle::new(
                        u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                        ColorFormat::CSS_LIGHT_CYAN,
                    ),
                    Alignment::Center,
                )
                .draw(&mut pixel_target)?;
            }
            target.draw_iter(self.state_text_pixels.iter().cloned())?;

            if self.asr_text_pixels.is_empty() {
                let mut pixel_target = PixelsTarget {
                    pixels: &mut self.asr_text_pixels,
                    bounding_box,
                };
                Text::with_alignment(
                    &self.asr_text,
                    asr_area_box.center(),
                    U8g2TextStyle::new(
                        u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                        ColorFormat::CSS_WHEAT,
                    ),
                    Alignment::Center,
                )
                .draw(&mut pixel_target)?;
            }
            target.draw_iter(self.asr_text_pixels.iter().cloned())?;

            if self.content_pixels.is_empty() {
                let mut pixel_target = PixelsTarget {
                    pixels: &mut self.content_pixels,
                    bounding_box,
                };
                let textbox_style = embedded_text::style::TextBoxStyleBuilder::new()
                    .height_mode(embedded_text::style::HeightMode::FitToText)
                    .alignment(embedded_text::alignment::HorizontalAlignment::Center)
                    .line_height(embedded_graphics::text::LineHeight::Percent(120))
                    .paragraph_spacing(16)
                    .build();

                embedded_text::TextBox::with_textbox_style(
                    &self.content,
                    content_area_box,
                    crate::ui::MyTextStyle(
                        U8g2TextStyle::new(
                            u8g2_fonts::fonts::u8g2_font_wqy16_t_gb2312,
                            ColorFormat::CSS_WHEAT,
                        ),
                        3,
                    ),
                    textbox_style,
                )
                .draw(&mut pixel_target)?;
            }
            target.draw_iter(self.content_pixels.iter().cloned())?;

            Ok(())
        }

        pub fn layout(bounding_box: Rectangle) -> (Rectangle, Rectangle, Rectangle) {
            let state_area_box = Rectangle::new(
                bounding_box.top_left,
                Size::new(bounding_box.size.width, 32),
            );

            let asr_area_box = Rectangle::new(
                bounding_box.top_left + Point::new(0, 32),
                Size::new(bounding_box.size.width, 32),
            );

            let content_height = bounding_box.size.height - 64;

            let content_area_box = Rectangle::new(
                bounding_box.top_left + Point::new(0, 64),
                Size::new(bounding_box.size.width, content_height),
            );

            (state_area_box, asr_area_box, content_area_box)
        }
    }

    pub fn new_chat_ui<const N: usize>(target: &mut FrameBuffer) -> anyhow::Result<ChatUI<N>> {
        let bounding_box = target.bounding_box();

        let header_area_box = Rectangle::new(
            bounding_box.center()
                - Point {
                    x: AVATAR_SIZE as i32 / 2,
                    y: AVATAR_SIZE as i32 / 2,
                },
            Size::new(AVATAR_SIZE, AVATAR_SIZE),
        );

        let (state_area_box, asr_area_box, content_area_box) = ChatUI::<N>::layout(bounding_box);
        let state_style = PrimitiveStyleBuilder::new()
            .stroke_color(ColorFormat::CSS_DARK_BLUE)
            .stroke_width(1)
            .fill_color(ColorFormat::CSS_DARK_BLUE)
            .build();

        let pixels = crate::ui::get_background_pixels(target, state_area_box, state_style, 0.5);
        target.draw_iter(pixels)?;

        let asr_style = PrimitiveStyleBuilder::new()
            .stroke_color(ColorFormat::CSS_DARK_CYAN)
            .stroke_width(1)
            .fill_color(ColorFormat::CSS_DARK_CYAN)
            .build();

        let pixels = crate::ui::get_background_pixels(target, asr_area_box, asr_style, 0.5);
        target.draw_iter(pixels)?;

        let content_style = PrimitiveStyleBuilder::new()
            .stroke_color(ColorFormat::CSS_BLACK)
            .stroke_width(5)
            .fill_color(ColorFormat::CSS_BLACK)
            .build();
        let pixels = crate::ui::get_background_pixels(target, content_area_box, content_style, 0.5);
        target.draw_iter(pixels)?;

        target.background_buffers.clone_from(&target.buffers);

        let avatar = DynamicImage::new_from_gif(header_area_box, crate::ui::AVATAR_GIF)?;

        Ok(ChatUI::new(avatar))
    }

    pub struct ConfiguresUI {
        qr_area: ImageArea,
        info: String,
    }

    impl ConfiguresUI {
        pub fn new(
            bounding_box: Rectangle,
            qr_content: &str,
            info: String,
        ) -> anyhow::Result<Self> {
            let height = bounding_box.size.height;
            let qr_area_box = Rectangle::new(
                bounding_box.top_left + Point::new(0, height as i32 / 3),
                Size::new(bounding_box.size.width, 2 * height / 3),
            );

            Ok(Self {
                qr_area: ImageArea::new_from_qr_code(qr_area_box, qr_content)?,
                info,
            })
        }

        pub fn set_info(&mut self, info: String) {
            self.info = info;
        }
    }

    impl Drawable for ConfiguresUI {
        type Color = ColorFormat;

        type Output = ();

        fn draw<D>(&self, target: &mut D) -> Result<Self::Output, D::Error>
        where
            D: DrawTarget<Color = Self::Color>,
        {
            let info_area_box = Rectangle::new(
                target.bounding_box().top_left,
                Size::new(
                    target.bounding_box().size.width,
                    target.bounding_box().size.height / 3,
                ),
            );

            Text::with_alignment(
                &self.info,
                info_area_box.center(),
                U8g2TextStyle::new(
                    u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                    ColorFormat::CSS_WHEAT,
                ),
                Alignment::Center,
            )
            .draw(target)?;

            target.draw_iter(self.qr_area.image_data.iter().cloned())?;

            Ok(())
        }
    }
}
