/**
 ****************************************************************************************************
 * @file        lcd.c
 * @author      正点原子团队(ALIENTEK)
 * @version     V1.0
 * @date        2024-06-25
 * @brief       LCD(MCU屏) 驱动代码
 *
 * @license     Copyright (c) 2020-2032, 广州市星翼电子科技有限公司
 ****************************************************************************************************
 * @attention
 *
 * 实验平台:正点原子 ESP32S3 BOX 开发板
 * 在线视频:www.yuanzige.com
 * 技术论坛:www.openedv.com
 * 公司网址:www.alientek.com
 * 购买地址:openedv.taobao.com
 *
 ****************************************************************************************************
 */

#include "lcd.h"
// #include "lcdfont.h"

static const char *TAG = "LCD";
esp_lcd_panel_handle_t panel_handle = NULL; /* LCD句柄 */
uint32_t g_back_color = 0xFFFF;
lcd_obj_t lcd_dev;

/**
 * @brief       以一种颜色清空LCD屏
 * @param       color 清屏颜色
 * @retval      无
 */
void lcd_clear(uint16_t color)
{
    uint16_t *buffer = heap_caps_malloc(lcd_dev.width * sizeof(uint16_t), MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);

    if (NULL == buffer)
    {
        ESP_LOGE(TAG, "Memory for bitmap is not enough");
    }
    else
    {
        for (uint16_t i = 0; i < lcd_dev.width; i++)
        {
            buffer[i] = color;
        }

        for (uint16_t y = 0; y < lcd_dev.height; y++)
        {
            esp_lcd_panel_draw_bitmap(panel_handle, 0, y, lcd_dev.width, y + 1, buffer);
        }

        heap_caps_free(buffer);
    }
}

/**
 * @brief       在指定区域内填充单个颜色
 * @note        此函数仅支持uint16_t,RGB565格式的颜色数组填充.
 *              (sx,sy),(ex,ey):填充矩形对角坐标,区域大小为:(ex - sx + 1) * (ey - sy + 1)
 *              注意:sx,ex,不能大于lcd_dev.width - 1; sy,ey,不能大于lcd_dev.height - 1
 * @param       sx,sy:起始坐标
 * @param       ex,ey:结束坐标
 * @param       color:要填充的颜色
 * @retval      无
 */
void lcd_fill(uint16_t sx, uint16_t sy, uint16_t ex, uint16_t ey, uint16_t color)
{
    /* 确保坐标在合法范围内 */
    if (sx >= lcd_dev.width || sy >= lcd_dev.height || ex > lcd_dev.width || ey > lcd_dev.height || sx >= ex || sy >= ey)
    {
        ESP_LOGE("TAG", "Invalid fill area");
        return;
    }

    /* 计算填充区域宽度 */
    uint16_t width = ex - sx;
    uint16_t height = ey - sy;

    /* 分配内存 */
    uint16_t *buffer = heap_caps_malloc(width * sizeof(uint16_t), MALLOC_CAP_INTERNAL);

    if (NULL == buffer)
    {
        ESP_LOGE(TAG, "Memory for bitmap is not enough");
    }
    else
    {
        /* 填充颜色 */
        for (uint16_t i = 0; i < width; i++)
        {
            buffer[i] = color;
        }

        /* 绘制填充区域 */
        for (uint16_t y = 0; y < height; y++)
        {
            esp_lcd_panel_draw_bitmap(panel_handle, sx, sy + y, width, sy + y + 1, buffer);
        }

        /* 释放内存 */
        heap_caps_free(buffer);
    }
}

/**
 * @brief       在指定区域内填充指定颜色块
 * @note        此函数仅支持uint16_t,RGB565格式的颜色数组填充.
 *              (sx,sy),(ex,ey):填充矩形对角坐标,区域大小为:(ex - sx + 1) * (ey - sy + 1)
 *              注意:sx,ex,不能大于lcd_dev.width - 1; sy,ey,不能大于lcd_dev.height - 1
 * @param       sx,sy:起始坐标
 * @param       ex,ey:结束坐标
 * @param       color:填充的颜色数组首地址
 * @retval      无
 */
void lcd_color_fill(uint16_t sx, uint16_t sy, uint16_t ex, uint16_t ey, uint16_t *color)
{
    /* 确保坐标在合法范围内 */
    if (sx >= lcd_dev.width || sy >= lcd_dev.height || ex > lcd_dev.width || ey > lcd_dev.height || sx >= ex || sy >= ey)
    {
        ESP_LOGE("TAG", "Invalid fill area");
        return;
    }

    /* 计算填充区域的宽度 */
    uint16_t width = ex - sx;
    uint16_t height = ey - sy;
    uint32_t buf_index = 0;

    uint16_t *buffer = heap_caps_malloc(width * sizeof(uint16_t), MALLOC_CAP_INTERNAL);

    for (uint16_t y_index = 0; y_index < height; y_index++)
    {
        for (uint16_t x_index = 0; x_index < width; x_index++)
        {
            buffer[x_index] = color[buf_index];
            buf_index++;
        }

        for (uint16_t i = 0; i < width; i += 80)
        {
            esp_lcd_panel_draw_bitmap(panel_handle, sx + i, sy + y_index, sx + i + 80, sy + y_index + 1, &buffer[i]);
        }
    }
    /* 释放内存 */
    heap_caps_free(buffer);
}

/**
 * @brief       画一个矩形
 * @param       x1,y1   起点坐标
 * @param       x2,y2   终点坐标
 * @param       color 填充颜色
 * @retval      无
 */
void lcd_draw_rectangle(uint16_t x0, uint16_t y0, uint16_t x1, uint16_t y1, uint16_t color)
{
    lcd_draw_line(x0, y0, x1, y0, color);
    lcd_draw_line(x0, y0, x0, y1, color);
    lcd_draw_line(x0, y1, x1, y1, color);
    lcd_draw_line(x1, y0, x1, y1, color);
}

/**
 * @brief       画圆
 * @param       x0,y0:圆中心坐标
 * @param       r    :半径
 * @param       color:圆的颜色
 * @retval      无
 */
void lcd_draw_circle(uint16_t x0, uint16_t y0, uint8_t r, uint16_t color)
{
    int a, b;
    int di;
    a = 0;
    b = r;
    di = 3 - (r << 1); /* 判断下个点位置的标志 */

    while (a <= b)
    {
        lcd_draw_point(x0 + a, y0 - b, color); /* 5 */
        lcd_draw_point(x0 + b, y0 - a, color); /* 0 */
        lcd_draw_point(x0 + b, y0 + a, color); /* 4 */
        lcd_draw_point(x0 + a, y0 + b, color); /* 6 */
        lcd_draw_point(x0 - a, y0 + b, color); /* 1 */
        lcd_draw_point(x0 - b, y0 + a, color);
        lcd_draw_point(x0 - a, y0 - b, color); /* 2 */
        lcd_draw_point(x0 - b, y0 - a, color); /* 7 */
        a++;

        /* 使用Bresenham算法画圆 */
        if (di < 0)
        {
            di += 4 * a + 6;
        }
        else
        {
            di += 10 + 4 * (a - b);
            b--;
        }
    }
}

/**
 * @brief       LCD显示方向设置
 * @param       dir:0,竖屏；1,横屏
 * @retval      无
 */
void lcd_display_dir(uint8_t dir)
{
    lcd_dev.dir = dir; /* 显示方向 */

    if (lcd_dev.dir == 0) /* 竖屏 */
    {
        lcd_dev.width = lcd_dev.pheight;
        lcd_dev.height = lcd_dev.pwidth;
        esp_lcd_panel_swap_xy(panel_handle, false);       /* 交换X和Y轴 */
        esp_lcd_panel_mirror(panel_handle, false, false); /* 对屏幕的Y轴进行镜像处理 */
    }
    else if (lcd_dev.dir == 1) /* 横屏 */
    {
        lcd_dev.width = lcd_dev.pwidth;
        lcd_dev.height = lcd_dev.pheight;
        esp_lcd_panel_swap_xy(panel_handle, true);       /* 不需要交换X和Y轴 */
        esp_lcd_panel_mirror(panel_handle, true, false); /* 对屏幕的XY轴不进行镜像处理 */
    }
}

/**
 * @brief       lcd画点函数
 * @param       x,y     :写入坐标
 * @param       color   :颜色值
 * @retval      无
 */
void lcd_draw_point(uint16_t x, uint16_t y, uint16_t color)
{
    esp_lcd_panel_draw_bitmap(panel_handle, x, y, x + 1, y + 1, (uint16_t *)&color);
}

/**
 * @brief       LCD初始化
 * @param       lcd_config:LCD配置信息
 * @retval      无
 */
void lcd_init(lcd_cfg_t lcd_config)
{
    gpio_config_t gpio_init_struct = {0};
    esp_lcd_panel_io_handle_t io_handle = NULL;

    lcd_dev.wr = LCD_NUM_WR; /* 配置WR引脚 */
    lcd_dev.cs = LCD_NUM_CS; /* 配置CS引脚 */
    lcd_dev.dc = LCD_NUM_DC; /* 配置DC引脚 */
    lcd_dev.rd = LCD_NUM_RD; /* 配置RD引脚 */

    lcd_dev.pwidth = 320;  /* 面板宽度,单位:像素 */
    lcd_dev.pheight = 240; /* 面板高度,单位:像素 */

    /* 配置RD引脚 */
    gpio_init_struct.intr_type = GPIO_INTR_DISABLE;        /* 失能引脚中断 */
    gpio_init_struct.mode = GPIO_MODE_INPUT_OUTPUT;        /* 配置输出模式 */
    gpio_init_struct.pin_bit_mask = 1ull << lcd_dev.rd;    /* 配置引脚位掩码 */
    gpio_init_struct.pull_down_en = GPIO_PULLDOWN_DISABLE; /* 失能下拉 */
    gpio_init_struct.pull_up_en = GPIO_PULLUP_ENABLE;      /* 使能下拉 */
    gpio_config(&gpio_init_struct);                        /* 引脚配置 */
    gpio_set_level(lcd_dev.rd, 1);                         /* RD管脚拉高 */

    esp_lcd_i80_bus_handle_t i80_bus = NULL;
    esp_lcd_i80_bus_config_t bus_config = {
        /* 初始化80并口总线 */
        .clk_src = LCD_CLK_SRC_DEFAULT,
        .dc_gpio_num = lcd_dev.dc,
        .wr_gpio_num = lcd_dev.wr,
        .data_gpio_nums = {
            GPIO_LCD_D0,
            GPIO_LCD_D1,
            GPIO_LCD_D2,
            GPIO_LCD_D3,
            GPIO_LCD_D4,
            GPIO_LCD_D5,
            GPIO_LCD_D6,
            GPIO_LCD_D7,
        },
        .bus_width = 8,
        .max_transfer_bytes = lcd_dev.pwidth * lcd_dev.pheight * sizeof(uint16_t),
        .psram_trans_align = 64,
        .sram_trans_align = 4,
    };
    ESP_ERROR_CHECK(esp_lcd_new_i80_bus(&bus_config, &i80_bus)); /* 新建80并口总线 */

    esp_lcd_panel_io_i80_config_t io_config = {
        /* 80并口配置 */
        .cs_gpio_num = lcd_dev.cs,
        .pclk_hz = (10 * 1000 * 1000),
        .trans_queue_depth = 10,
        .dc_levels = {
            .dc_idle_level = 0,
            .dc_cmd_level = 0,
            .dc_dummy_level = 0,
            .dc_data_level = 1,
        },
        .flags = {
            .swap_color_bytes = 1,
        },
        .on_color_trans_done = lcd_config.notify_flush_ready,
        .user_ctx = lcd_config.user_ctx,
        .lcd_cmd_bits = 8,
        .lcd_param_bits = 8,
    };
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_i80(i80_bus, &io_config, &io_handle));

    esp_lcd_panel_dev_config_t panel_config = {
        .reset_gpio_num = LCD_NUM_RST,
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = 16,
    };
    ESP_ERROR_CHECK(esp_lcd_new_panel_st7789(io_handle, &panel_config, &panel_handle));

    esp_lcd_panel_reset(panel_handle);              /* 复位屏幕 */
    esp_lcd_panel_init(panel_handle);               /* 初始化屏幕 */
    esp_lcd_panel_invert_color(panel_handle, true); /* 开启颜色反转 */
    esp_lcd_panel_set_gap(panel_handle, 0, 0);      /* 设置XY偏移 */
    esp_lcd_panel_io_tx_param(io_handle, 0x36, (uint8_t[]){0}, 1);
    esp_lcd_panel_io_tx_param(io_handle, 0x3A, (uint8_t[]){0x65}, 1);
    lcd_display_dir(1);                                             /* 设置屏幕方向 */
    ESP_ERROR_CHECK(esp_lcd_panel_disp_on_off(panel_handle, true)); /* 启动屏幕 */
    lcd_clear(WHITE);                                               /* 默认填充白色 */
    LCD_BL(1);                                                      /* 打开背光 */
}
