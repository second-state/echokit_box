use esp_idf_svc::{
    hal::{gpio::*, i2c::I2C0, i2s::I2S0},
    sys::EspError,
};

const AUDIO_STACK_SIZE: usize = 15 * 1024;
pub const AFE_AEC_OFFSET: usize = 512;

pub fn afe_config(afe_config: &mut esp_idf_svc::sys::esp_sr::afe_config_t) {
    afe_config.agc_init = true;
    afe_config.agc_mode = esp_idf_svc::sys::esp_sr::afe_agc_mode_t_AFE_AGC_MODE_WEBRTC;
    afe_config.afe_linear_gain = 1.0;
    afe_config.ns_init = false;
}

pub fn audio_init(_i2c: I2C0, _sda: Gpio48, _scl: Gpio45) {
    const SAMPLE_RATE: u32 = 16000;

    unsafe {
        use esp_idf_svc::sys::hal_driver;

        hal_driver::myiic_init();
        hal_driver::xl9555_init();
        hal_driver::es8311_init(SAMPLE_RATE as i32);
        hal_driver::xl9555_pin_write(hal_driver::SPK_CTRL_IO as _, 1);
        hal_driver::es8311_set_voice_volume(70);
        hal_driver::es8311_set_mic_gain(hal_driver::es8311_mic_gain_t_ES8311_MIC_GAIN_24DB);
        hal_driver::es8311_set_voice_mute(0); /* 打开DAC */
    }
}

pub fn start_audio_workers(
    i2s: I2S0,
    bclk: Gpio21,
    din: Gpio47,
    dout: Gpio14,
    ws: Gpio13,

    rx: crate::audio::PlayerRx,
    tx: crate::audio::EventTx,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    let worker = crate::audio::BoxAudioWorker {
        i2s,
        bclk: bclk.into(),
        din: din.into(),
        dout: dout.into(),
        ws: ws.into(),
        mclk: None,
    };

    let r = std::thread::Builder::new()
        .stack_size(AUDIO_STACK_SIZE)
        .spawn(move || {
            log::info!(
                "Starting audio worker thread in core {:?}",
                esp_idf_svc::hal::cpu::core()
            );
            let r = worker.run(rx, tx);
            if let Err(e) = r {
                log::error!("Audio worker error: {:?}", e);
            }
        })
        .map_err(|e| anyhow::anyhow!("Failed to spawn audio worker thread: {:?}", e))?;

    Ok(r)
}

pub fn start_btn_worker(
    rt: &tokio::runtime::Runtime,
    int_gpio: Gpio3,
    evt_tx: crate::audio::EventTx,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let mut int_gpio = esp_idf_svc::hal::gpio::PinDriver::input(int_gpio)?;
    int_gpio.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    int_gpio.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::NegEdge)?;

    Ok(rt.spawn(async move {
        loop {
            let r = int_gpio.wait_for_falling_edge().await;
            if let Err(e) = r {
                log::error!("Volume button wait error: {:?}", e);
                continue;
            }

            unsafe {
                let k = esp_idf_svc::sys::hal_driver::xl9555_key_scan(0) as u32;
                match k {
                    esp_idf_svc::sys::hal_driver::KEY0_PRES => {
                        log::info!("Volume up button pressed");
                        let r = evt_tx
                            .send(crate::app::Event::Event(crate::app::Event::VOL_UP))
                            .await;
                        if r.is_err() {
                            log::error!("Failed to send volume up event: {:?}", r.err());
                        }
                    }
                    esp_idf_svc::sys::hal_driver::KEY1_PRES => {
                        log::info!("Volume down button pressed");
                        let r = evt_tx
                            .send(crate::app::Event::Event(crate::app::Event::VOL_DOWN))
                            .await;
                        if r.is_err() {
                            log::error!("Failed to send volume down event: {:?}", r.err());
                        }
                    }
                    _ => {
                        log::debug!("Unknown key code: {}", k);
                    }
                }
            }
        }
    }))
}

pub fn set_volum(vol: u8) {
    let v = match vol {
        0..5 => vol as i32 * 50 / 5 + 20,
        _ => 70,
    };

    unsafe {
        esp_idf_svc::sys::hal_driver::es8311_set_voice_volume(v);
    }
}

pub const DISPLAY_WIDTH: usize = 320;
pub const DISPLAY_HEIGHT: usize = 240;

pub fn lcd_init(
    _cs: Gpio1,
    _dc: Gpio2,
    _rd: Gpio41,
    _wr: Gpio42,
    _lcd_data: (
        Gpio40,
        Gpio39,
        Gpio38,
        Gpio12,
        Gpio11,
        Gpio10,
        Gpio9,
        Gpio46,
    ),
) -> Result<(), EspError> {
    use esp_idf_svc::sys::hal_driver;
    unsafe {
        let config: hal_driver::lcd_cfg_t = std::mem::zeroed();
        hal_driver::lcd_init(config);
    }
    Ok(())
}

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

    type FrameBufferChunk8x12 = Framebuffer<
        ColorFormat,
        RawU16,
        LittleEndian,
        8,
        12,
        { buffer_size::<ColorFormat>(8, 12) },
    >;

    pub type DisplayBuffer = BoxFrameBuffer;

    type FrameMask = [u8; (DISPLAY_WIDTH / 8) * (DISPLAY_HEIGHT / 12)];

    pub struct BoxFrameBuffer {
        buffers: Vec<FrameBufferChunk8x12>, //[FrameBufferChunk8x12; (DISPLAY_WIDTH / 8) * (DISPLAY_HEIGHT / 12)],
        background_buffers: Vec<FrameBufferChunk8x12>, //[FrameBufferChunk8x12; (DISPLAY_WIDTH / 8) * (DISPLAY_HEIGHT / 12)],
        diff_indexs: Vec<usize>,
        resume_indexs: Vec<usize>,
        draw_mask: FrameMask,
    }

    impl Dimensions for BoxFrameBuffer {
        fn bounding_box(&self) -> Rectangle {
            Rectangle::new(
                Point::new(0, 0),
                Size::new(DISPLAY_WIDTH as u32, DISPLAY_HEIGHT as u32),
            )
        }
    }

    impl DrawTarget for BoxFrameBuffer {
        type Color = ColorFormat;
        type Error = core::convert::Infallible;

        fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = embedded_graphics::Pixel<Self::Color>>,
        {
            for embedded_graphics::Pixel(coord, color) in pixels {
                if coord.x < 0
                    || coord.x >= DISPLAY_WIDTH as i32
                    || coord.y < 0
                    || coord.y >= DISPLAY_HEIGHT as i32
                {
                    continue;
                }

                let x = coord.x as usize;
                let y = coord.y as usize;

                let chunk_x = x / 8;
                let chunk_y = y / 12;
                let chunk_index = chunk_y * (DISPLAY_WIDTH / 8) + chunk_x;

                let local_x = x % 8;
                let local_y = y % 12;

                if self.draw_mask[chunk_index] == 0 {
                    self.diff_indexs.push(chunk_index);
                    self.draw_mask[chunk_index] = 1;
                }

                self.buffers[chunk_index].set_pixel(
                    embedded_graphics::prelude::Point::new(local_x as i32, local_y as i32),
                    color,
                );
            }

            Ok(())
        }
    }

    impl GetPixel for BoxFrameBuffer {
        type Color = ColorFormat;

        fn pixel(&self, point: Point) -> Option<Self::Color> {
            if point.x < 0
                || point.x >= DISPLAY_WIDTH as i32
                || point.y < 0
                || point.y >= DISPLAY_HEIGHT as i32
            {
                return None;
            }

            let x = point.x as usize;
            let y = point.y as usize;

            let chunk_x = x / 8;
            let chunk_y = y / 12;
            let chunk_index = chunk_y * (DISPLAY_WIDTH / 8) + chunk_x;

            let local_x = x % 8;
            let local_y = y % 12;

            self.buffers[chunk_index].pixel(embedded_graphics::prelude::Point::new(
                local_x as i32,
                local_y as i32,
            ))
        }
    }

    impl DisplayTargetDrive for BoxFrameBuffer {
        fn new(color: ColorFormat) -> Self {
            let mut s = Self {
                buffers: vec![Framebuffer::new(); (DISPLAY_WIDTH / 8) * (DISPLAY_HEIGHT / 12)],
                background_buffers: vec![
                    Framebuffer::new();
                    (DISPLAY_WIDTH / 8) * (DISPLAY_HEIGHT / 12)
                ],
                diff_indexs: Vec::new(),
                resume_indexs: Vec::new(),
                draw_mask: [0; (DISPLAY_WIDTH / 8) * (DISPLAY_HEIGHT / 12)],
            };

            for buffer in s.buffers.iter_mut() {
                buffer.clear(color).unwrap();
            }

            for buffer in s.background_buffers.iter_mut() {
                buffer.clear(color).unwrap();
            }

            s
        }

        fn flush(&mut self) -> anyhow::Result<()> {
            let now = std::time::Instant::now();
            unsafe {
                let panel_handle = std::mem::transmute(esp_idf_svc::sys::hal_driver::panel_handle);

                for i in self.diff_indexs.iter().chain(self.resume_indexs.iter()) {
                    let i = *i;
                    let x_start = ((i % (DISPLAY_WIDTH / 8)) * 8) as i32;
                    let y_start = ((i / (DISPLAY_WIDTH / 8)) * 12) as i32;
                    let x_end = x_start + 8;
                    let y_end = y_start + 12;

                    // DEBUG
                    // self.buffers[i].clear(ColorFormat::CSS_GOLD).unwrap();

                    let color_data = self.buffers[i].data();
                    let size = color_data.len();

                    let lcd_dma: *mut u8 = esp_idf_svc::sys::hal_driver::lcd_dma_buffer as *mut u8;
                    lcd_dma.copy_from(color_data.as_ptr() as *const u8, size);

                    let e = esp_idf_svc::sys::esp_lcd_panel_draw_bitmap(
                        panel_handle,
                        x_start,
                        y_start,
                        x_end,
                        y_end,
                        lcd_dma as *const _,
                    );
                    if e != 0 {
                        log::warn!("flush_display error: {}", e);
                    }

                    if self.draw_mask[i] != 0 {
                        self.draw_mask[i] = 0;
                        self.buffers[i].clone_from(&self.background_buffers[i]);
                    }
                }

                log::info!(
                    "Display flush took {:?} for {} chunks, {} resumed",
                    now.elapsed(),
                    self.diff_indexs.len(),
                    self.resume_indexs.len()
                );

                self.diff_indexs.clear();
                self.resume_indexs.clear();
            }
            Ok(())
        }

        fn fix_background(&mut self) -> anyhow::Result<()> {
            self.background_buffers.clone_from(&self.buffers);
            Ok(())
        }
    }

    impl BoxFrameBuffer {
        fn resume_chunks(&mut self, chunks: &[usize]) {
            for &i in chunks {
                if self.draw_mask[i] == 0 {
                    self.resume_indexs.push(i);
                }
            }
        }
    }

    pub struct ChatUI<const N: usize> {
        state_text: String,
        state_text_updated: bool,
        state_chunks: Vec<usize>,

        asr_text: String,
        asr_text_updated: bool,
        asr_text_chunks: Vec<usize>,

        content: String,
        content_updated: bool,
        content_chunks: Vec<usize>,

        avatar: DynamicImage<N>,
        avatar_updated: bool,
        avatar_chunks: Vec<usize>,
    }

    impl<const N: usize> ChatUI<N> {
        pub fn new(avatar: DynamicImage<N>) -> Self {
            Self {
                state_text: String::new(),
                state_text_updated: false,
                state_chunks: Vec::new(),

                asr_text: String::new(),
                asr_text_updated: false,
                asr_text_chunks: Vec::new(),

                content: String::new(),
                content_updated: false,
                content_chunks: Vec::new(),

                avatar: avatar,
                avatar_updated: true,
                avatar_chunks: Vec::new(),
            }
        }

        pub fn set_state(&mut self, text: String) {
            if self.state_text != text {
                self.state_text = text;
                self.state_text_updated = true;
            }
        }

        pub fn set_asr(&mut self, text: String) {
            if self.asr_text != text {
                self.asr_text = text;
                self.asr_text_updated = true;
            }
        }

        pub fn set_text(&mut self, text: String) {
            if self.content != text {
                self.content = text;
                self.content_updated = true;
            }
        }

        pub fn set_avatar_index(&mut self, index: usize) {
            self.avatar.set_index(index);
            self.avatar_updated = true;
        }

        pub fn clear_update_flags(&mut self) {
            self.state_text_updated = false;
            self.asr_text_updated = false;
            self.content_updated = false;
            self.avatar_updated = false;
        }

        pub fn render_to_target(&mut self, target: &mut BoxFrameBuffer) -> anyhow::Result<()> {
            let bounding_box = target.bounding_box();
            let (state_area_box, asr_area_box, content_area_box) = Self::layout(bounding_box);

            log::info!(
                "draw ChatUI {} {} {} {}",
                self.state_text_updated,
                self.asr_text_updated,
                self.content_updated,
                self.avatar_updated
            );

            let mut start_i = 0;

            if self.state_text_updated {
                Text::with_alignment(
                    &self.state_text,
                    state_area_box.center(),
                    U8g2TextStyle::new(
                        u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                        ColorFormat::CSS_LIGHT_CYAN,
                    ),
                    Alignment::Center,
                )
                .draw(target)?;
                target.resume_chunks(&self.state_chunks);
                self.state_chunks = target.diff_indexs.clone();
                start_i = self.state_chunks.len();
            }

            if self.asr_text_updated {
                Text::with_alignment(
                    &self.asr_text,
                    asr_area_box.center(),
                    U8g2TextStyle::new(
                        u8g2_fonts::fonts::u8g2_font_wqy12_t_gb2312a,
                        ColorFormat::CSS_WHEAT,
                    ),
                    Alignment::Center,
                )
                .draw(target)?;
                target.resume_chunks(&self.asr_text_chunks);
                self.asr_text_chunks = target.diff_indexs[start_i..].to_vec();
                start_i += self.asr_text_chunks.len();
            }

            if self.content_updated {
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
                .draw(target)?;
                target.resume_chunks(&self.content_chunks);
                self.content_chunks = target.diff_indexs[start_i..].to_vec();
                start_i += self.content_chunks.len();
            }

            if self.avatar_updated {
                self.avatar.render(target)?;
                target.resume_chunks(&self.avatar_chunks);
                self.avatar_chunks = target.diff_indexs[start_i..].to_vec();
            }

            self.clear_update_flags();

            Ok(())
        }

        pub fn layout(bounding_box: Rectangle) -> (Rectangle, Rectangle, Rectangle) {
            let state_area_box = Rectangle::new(
                bounding_box.top_left + Point::new(96, 0),
                Size::new(bounding_box.size.width - 96, 32),
            );

            let asr_area_box = Rectangle::new(
                bounding_box.top_left + Point::new(96, 32),
                Size::new(bounding_box.size.width - 96, 64),
            );

            let content_area_box = Rectangle::new(
                bounding_box.top_left + Point::new(0, 32 + 64),
                Size::new(bounding_box.size.width, bounding_box.size.height - 32 - 64),
            );

            (state_area_box, asr_area_box, content_area_box)
        }
    }

    pub fn new_chat_ui<const N: usize>(target: &mut BoxFrameBuffer) -> anyhow::Result<ChatUI<N>> {
        let bounding_box = target.bounding_box();
        let avatar_area_box = Rectangle::new(bounding_box.top_left, Size::new(96, 96));

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

        let pixels = crate::ui::get_background_pixels(target, asr_area_box, asr_style, 0.15);
        target.draw_iter(pixels)?;

        let content_style = PrimitiveStyleBuilder::new()
            .stroke_color(ColorFormat::CSS_BLACK)
            .stroke_width(5)
            .fill_color(ColorFormat::CSS_BLACK)
            .build();
        let pixels =
            crate::ui::get_background_pixels(target, content_area_box, content_style, 0.25);
        target.draw_iter(pixels)?;

        target.background_buffers.clone_from(&target.buffers);

        target.flush()?;

        let avatar = DynamicImage::new_from_gif(avatar_area_box, crate::ui::AVATAR_GIF)?;
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
                info: info.to_string(),
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

pub fn flush_display(color_data: &[u8], x_start: i32, y_start: i32, x_end: i32, y_end: i32) -> i32 {
    // if write area size > 80, lcd will display wrong data

    debug_assert_eq!(
        x_end - x_start,
        DISPLAY_WIDTH as i32,
        "x_end - x_start must be equal to DISPLAY_WIDTH"
    );
    unsafe {
        esp_idf_svc::sys::hal_driver::lcd_color_fill(
            x_start as u16,
            y_start as u16,
            x_end as u16,
            y_end as u16,
            color_data.as_ptr() as _,
        );
        0
    }
}

#[macro_export]
macro_rules! start_hal {
    ($peripherals:ident, $evt_tx:ident) => {{
        crate::boards::atom_box::audio_init(
            $peripherals.i2c0,
            $peripherals.pins.gpio48,
            $peripherals.pins.gpio45,
        );
        crate::boards::atom_box::lcd_init(
            $peripherals.pins.gpio1,
            $peripherals.pins.gpio2,
            $peripherals.pins.gpio41,
            $peripherals.pins.gpio42,
            (
                $peripherals.pins.gpio40,
                $peripherals.pins.gpio39,
                $peripherals.pins.gpio38,
                $peripherals.pins.gpio12,
                $peripherals.pins.gpio11,
                $peripherals.pins.gpio10,
                $peripherals.pins.gpio9,
                $peripherals.pins.gpio46,
            ),
        )?;
    }};
}

#[macro_export]
macro_rules! start_audio_workers {
    ($peripherals:ident, $rx:expr, $evt_tx:expr, $tokio_rt:expr) => {{
        crate::boards::atom_box::start_audio_workers(
            $peripherals.i2s0,
            $peripherals.pins.gpio21,
            $peripherals.pins.gpio47,
            $peripherals.pins.gpio14,
            $peripherals.pins.gpio13,
            $rx,
            $evt_tx,
        )?;
        crate::boards::atom_box::start_btn_worker($tokio_rt, $peripherals.pins.gpio3, $evt_tx)?;
    }};
}
