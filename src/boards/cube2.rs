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
    vol_up_btn: Gpio40,
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

pub fn init_spi(_spi: SPI3, mosi: Gpio10, clk: Gpio9) -> Result<(), EspError> {
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

pub fn init_lcd(cs: Gpio14, dc: Gpio8, rst: Gpio18) -> Result<(), EspError> {
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

#[cfg(feature = "exio")]
pub fn touch_switch_init(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
) -> anyhow::Result<()> {
    use crate::peripheral::exio::emakefun_exio::*;
    // Set all pins to input mode
    set_gpio_mode(i2c, 0x24, GpioPin::E0, GpioMode::InputPullDown)?;
    set_gpio_mode(i2c, 0x24, GpioPin::E1, GpioMode::InputPullDown)?;

    Ok(())
}

#[cfg(feature = "exio")]
pub fn touch_switch_loop(
    i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'static>,
    evt_tx: &crate::audio::EventTx,
) -> anyhow::Result<()> {
    use crate::peripheral::exio::emakefun_exio::*;

    static E0: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    static E1: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

    // Read pin levels
    let e0_level = read_gpio_level(i2c, 0x24, GpioPin::E0)?;
    let e1_level = read_gpio_level(i2c, 0x24, GpioPin::E1)?;

    if e0_level != E0.load(std::sync::atomic::Ordering::SeqCst) {
        E0.store(e0_level, std::sync::atomic::Ordering::SeqCst);
        if e0_level == 1 {
            log::info!("Touch switch E0 pressed");
        } else {
            log::info!("Touch switch E0 not pressed");
        }
        if let Err(_) = evt_tx.blocking_send(crate::app::Event::Event(crate::app::Event::K0)) {
            log::error!("Failed to send k0 event");
        }
    }

    if e1_level != E1.load(std::sync::atomic::Ordering::SeqCst) {
        E1.store(e1_level, std::sync::atomic::Ordering::SeqCst);
        if e1_level == 1 {
            log::info!("Touch switch E1 pressed");
        } else {
            log::info!("Touch switch E1 not pressed");
        }
        if let Err(_) =
            evt_tx.blocking_send(crate::app::Event::Event(crate::app::Event::VOL_SWITCH))
        {
            log::error!("Failed to send k0 event");
        }
    }

    Ok(())
}

#[macro_export]
macro_rules! start_hal {
    ($peripherals:ident, $evt_tx:ident) => {{
        crate::boards::cube2::init_spi(
            $peripherals.spi3,
            $peripherals.pins.gpio10,
            $peripherals.pins.gpio9,
        )?;
        crate::boards::cube2::init_lcd(
            $peripherals.pins.gpio14,
            $peripherals.pins.gpio8,
            $peripherals.pins.gpio18,
        )?;
        #[cfg(feature = "i2c")]
        {
            let config = esp_idf_svc::hal::i2c::config::Config::default()
                .baudrate(esp_idf_svc::hal::units::Hertz(40_000));

            let mut i2c_tasks: Vec<(crate::boards::I2CInitFn, crate::boards::I2CLoopFn)> = vec![];

            #[cfg(feature = "mfrc522")]
            {
                i2c_tasks.push((crate::boards::init_mfrc522, crate::boards::mfrc522_loop));
            }
            #[cfg(feature = "exio")]
            {
                i2c_tasks.push((
                    crate::boards::touch_switch_init,
                    crate::boards::touch_switch_loop,
                ));
            }

            if let Err(e) = crate::boards::init_i2c(
                config,
                $peripherals.i2c0,
                $peripherals.pins.gpio41.into(),
                $peripherals.pins.gpio42.into(),
                $evt_tx.clone(),
                i2c_tasks,
                8 * 1024,
                1000,
            ) {
                log::error!("Failed to initialize I2C: {:?}", e);
            }
        }
    }
    let _backlight = {
        let mut backlight = crate::boards::backlight_init($peripherals.pins.gpio13.into()).unwrap();
        crate::boards::set_backlight(&mut backlight, 70).unwrap();
        backlight
    };};
}

#[macro_export]
macro_rules! start_audio_workers {
    ($peripherals:ident, $rx:expr, $evt_tx:expr, $tokio_rt:expr) => {{
        crate::boards::cube2::start_audio_workers(
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

        crate::boards::cube2::start_btn_worker(
            $tokio_rt,
            $peripherals.pins.gpio40,
            $peripherals.pins.gpio39,
            $evt_tx,
        )?;
    }};
}
