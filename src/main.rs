use std::str::FromStr;

use esp_idf_svc::{sys as _, wifi::{ClientConfiguration, Configuration}};
use dht11::Dht11;
use crossbeam_channel::bounded;
use esp_idf_hal::{
    delay::{Ets, FreeRtos}, gpio::{AnyIOPin, AnyOutputPin, IOPin, InputOutput, PinDriver}, peripherals::Peripherals
};

use esp_idf_svc::{
    wifi::EspWifi,
    nvs::EspDefaultNvsPartition,
    eventloop::EspSystemEventLoop,
};

use esp_println::println;
use heapless::String;


static TEMP_STACK_SIZE:usize = 2000;
const WIFI_SSID:&'static str=env!("WIFI_SSID");
const WIFI_PW:&'static str=env!("WIFI_PW");


fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();
    

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let dht_pin = PinDriver::input_output_od(peripherals.pins.gpio5.downgrade())?;
    let mut dht11 = Dht11::new(dht_pin);
    let mut wifi_driver = EspWifi::new(
        peripherals.modem,
        sys_loop,
        Some(nvs)
    )?;

    let wifi_ssid: String<32> = String::from_str(WIFI_SSID).unwrap();
    let wifi_pw: String<64> = String::from_str(&WIFI_PW).unwrap();
    wifi_driver.set_configuration(&Configuration::Client(ClientConfiguration{
        ssid: wifi_ssid,
        password: wifi_pw,
        ..Default::default()
    }))?;
    wifi_driver.start()?;
    wifi_driver.connect()?;
    while !wifi_driver.is_connected()?{
        let config = wifi_driver.get_configuration()?;
        println!("Waiting for station {:?}", config);
    };
    println!("Should be connected now");
    let temp_thread = std::thread::Builder::new()
        .stack_size(TEMP_STACK_SIZE)
        .spawn(move||dht11_thread_fuction(&mut dht11));
    loop {
        
        // println!("THREAD");
        // println!("IP info: {:?}", wifi_driver.sta_netif().get_ip_info()?);

        FreeRtos::delay_ms(2000);
    }
}

fn dht11_thread_fuction(dht11: &mut Dht11<PinDriver<AnyIOPin, InputOutput>>){
    loop{
        let mut dht11_delay = Ets;
        match dht11.perform_measurement(&mut dht11_delay) {
            Ok(measurement) =>println!("{},{}",measurement.temperature as f32 / 10.0,measurement.humidity as f32 / 10.0),
            Err(e)=>println!("{:?}",e)
        }
        // println!("THREAD2");
        FreeRtos::delay_ms(1000);
    }
}