use esp_idf_svc::{
    hal::{
        gpio::*,
        i2s::{I2S0, I2S1},
        spi::SPI3,
    },
    sys::EspError,
};

const AUDIO_STACK_SIZE: usize = 15 * 1024;

pub fn start_audio_workers(
    out_i2s: I2S1,
    sck: Gpio5,
    din: Gpio6,
    dout: Gpio7,

    in_i2s: I2S0,
    ws: Gpio4,
    bclk: Gpio15,
    lrclk: Gpio16,

    rx: crate::audio::PlayerRx,
    tx: crate::audio::EventTx,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    let worker = crate::audio::BoardsAudioWorker {
        out_i2s,
        out_ws: lrclk.into(),
        out_clk: bclk.into(),
        dout: dout.into(),
        out_mclk: None,

        in_i2s,
        in_ws: ws.into(),
        in_clk: sck.into(),
        din: din.into(),
        in_mclk: None,
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
    vol_up_btn: Gpio38,
    vol_down_btn: Gpio39,
    evt_tx: crate::audio::EventTx,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let mut vol_up_btn = esp_idf_svc::hal::gpio::PinDriver::input(vol_up_btn)?;
    vol_up_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    vol_up_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

    let mut vol_down_btn = esp_idf_svc::hal::gpio::PinDriver::input(vol_down_btn)?;
    vol_down_btn.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    vol_down_btn.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

    Ok(rt.spawn(async move {
        loop {
            tokio::select! {
                _ = vol_up_btn.wait_for_falling_edge() => {
                    log::info!("Volume up button pressed");
                    let r = evt_tx.send(crate::app::Event::Event(crate::app::Event::VOL_UP)).await;
                    if let Err(e) = r {
                        log::error!("Failed to send volume up event: {:?}", e);
                    }
                }
                _ = vol_down_btn.wait_for_falling_edge() => {
                    log::info!("Volume down button pressed");
                    let r = evt_tx.send(crate::app::Event::Event(crate::app::Event::VOL_DOWN)).await;
                    if let Err(e) = r {
                        log::error!("Failed to send volume down event: {:?}", e);
                    }
                }
            }
        }
    }))
}

pub const DISPLAY_WIDTH: usize = 240;
pub const DISPLAY_HEIGHT: usize = 240;

static mut ESP_LCD_PANEL_HANDLE: esp_idf_svc::sys::esp_lcd_panel_handle_t = std::ptr::null_mut();

pub fn init_spi(_spi: SPI3, mosi: Gpio47, clk: Gpio21) -> Result<(), EspError> {
    use esp_idf_svc::hal::spi::Spi;
    use esp_idf_svc::sys::*;
    const GPIO_NUM_NC: i32 = -1;

    let mut buscfg = spi_bus_config_t::default();
    buscfg.__bindgen_anon_1.mosi_io_num = mosi.pin();
    buscfg.__bindgen_anon_2.miso_io_num = GPIO_NUM_NC;
    buscfg.sclk_io_num = clk.pin();
    buscfg.__bindgen_anon_3.quadwp_io_num = GPIO_NUM_NC;
    buscfg.__bindgen_anon_4.quadhd_io_num = GPIO_NUM_NC;
    buscfg.max_transfer_sz = (DISPLAY_WIDTH * DISPLAY_HEIGHT * std::mem::size_of::<u16>()) as i32;
    esp!(unsafe { spi_bus_initialize(SPI3::device(), &buscfg, spi_common_dma_t_SPI_DMA_CH_AUTO,) })
}

pub fn init_lcd(cs: Gpio41, dc: Gpio40, rst: Gpio45) -> Result<(), EspError> {
    use esp_idf_svc::sys::*;

    ::log::info!("Install panel IO");
    let mut panel_io: esp_lcd_panel_io_handle_t = std::ptr::null_mut();
    let mut io_config = esp_lcd_panel_io_spi_config_t::default();
    io_config.cs_gpio_num = cs.pin();
    io_config.dc_gpio_num = dc.pin();
    io_config.spi_mode = 3;
    io_config.pclk_hz = 40 * 1000 * 1000;
    io_config.trans_queue_depth = 10;
    io_config.lcd_cmd_bits = 8;
    io_config.lcd_param_bits = 8;
    esp!(unsafe {
        esp_lcd_new_panel_io_spi(spi_host_device_t_SPI3_HOST as _, &io_config, &mut panel_io)
    })?;

    ::log::info!("Install LCD driver");

    let mut panel_config = esp_lcd_panel_dev_config_t::default();
    let mut panel: esp_lcd_panel_handle_t = std::ptr::null_mut();

    panel_config.reset_gpio_num = rst.pin();
    panel_config.data_endian = lcd_rgb_data_endian_t_LCD_RGB_DATA_ENDIAN_LITTLE;
    panel_config.__bindgen_anon_1.rgb_ele_order = lcd_rgb_element_order_t_LCD_RGB_ELEMENT_ORDER_RGB;
    panel_config.bits_per_pixel = 16;

    esp!(unsafe { esp_lcd_new_panel_st7789(panel_io, &panel_config, &mut panel) })?;

    unsafe {
        ESP_LCD_PANEL_HANDLE = panel;
    }

    const DISPLAY_MIRROR_X: bool = false;
    const DISPLAY_MIRROR_Y: bool = false;
    const DISPLAY_SWAP_XY: bool = false;
    const DISPLAY_INVERT_COLOR: bool = true;

    ::log::info!("Reset LCD panel");
    unsafe {
        esp!(esp_lcd_panel_reset(panel))?;
        esp!(esp_lcd_panel_init(panel))?;
        esp!(esp_lcd_panel_invert_color(panel, DISPLAY_INVERT_COLOR))?;
        esp!(esp_lcd_panel_swap_xy(panel, DISPLAY_SWAP_XY))?;
        esp!(esp_lcd_panel_mirror(
            panel,
            DISPLAY_MIRROR_X,
            DISPLAY_MIRROR_Y
        ))?;
        esp!(esp_lcd_panel_disp_on_off(panel, true))?; /* 启动屏幕 */
    }

    Ok(())
}

pub fn flush_display(color_data: &[u8], x_start: i32, y_start: i32, x_end: i32, y_end: i32) -> i32 {
    unsafe {
        let e = esp_idf_svc::sys::esp_lcd_panel_draw_bitmap(
            ESP_LCD_PANEL_HANDLE,
            x_start,
            y_start,
            x_end,
            y_end,
            color_data.as_ptr().cast(),
        );
        if e != 0 {
            log::warn!("flush_display error: {}", e);
        }
        e
    }
}

#[macro_export]
macro_rules! start_hal {
    ($peripherals:ident, $evt_tx:ident) => {{
        crate::boards::base::init_spi(
            $peripherals.spi3,
            $peripherals.pins.gpio47,
            $peripherals.pins.gpio21,
        )?;
        crate::boards::base::init_lcd(
            $peripherals.pins.gpio41,
            $peripherals.pins.gpio40,
            $peripherals.pins.gpio45,
        )?;
        #[cfg(feature = "mfrc522")]
        {
            crate::boards::init_mfrc522(
                $peripherals.i2c0,
                $peripherals.pins.gpio14.into(),
                $peripherals.pins.gpio13.into(),
                $evt_tx.clone(),
            );
        }
        #[cfg(not(feature = "mfrc522"))]
        {
            log::info!("MFRC522 feature not enabled, skipping RFID initialization");
            $evt_tx
                .blocking_send(crate::app::Event::ServerUrl(String::new()))
                .unwrap_or_else(|e| {
                    log::error!("Failed to send ServerUrl event: {:?}", e);
                });
        }
    }};
}

#[macro_export]
macro_rules! start_audio_workers {
    ($peripherals:ident, $rx:expr, $evt_tx:expr, $tokio_rt:expr) => {{
        crate::boards::base::start_audio_workers(
            $peripherals.i2s1,
            $peripherals.pins.gpio5,
            $peripherals.pins.gpio6,
            $peripherals.pins.gpio7,
            $peripherals.i2s0,
            $peripherals.pins.gpio4,
            $peripherals.pins.gpio15,
            $peripherals.pins.gpio16,
            $rx,
            $evt_tx,
        )?;
        crate::boards::base::start_btn_worker(
            $tokio_rt,
            $peripherals.pins.gpio38,
            $peripherals.pins.gpio39,
            $evt_tx,
        )?;
    }};
}
