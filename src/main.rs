use std::str::FromStr;
use std::io::Write;
use std::net::TcpStream;
use std::sync::Arc;
use esp_idf_svc::{sys as _, wifi::{ClientConfiguration, Configuration}};
use dht11::Dht11;
use crossbeam_channel::bounded;
use esp_idf_hal::{
    delay::{Ets, FreeRtos}, gpio::{AnyIOPin, AnyOutputPin, IOPin, InputOutput, Pin, PinDriver}, peripherals::Peripherals, rmt::{config::TransmitConfig, FixedLengthSignal, PinState, Pulse, PulseTicks, TxRmtDriver, VariableLengthSignal}
};
use esp_idf_hal::rmt::*;
use esp_idf_hal :: sys :: rmt_encoder_t;
use esp_idf_svc::{
    wifi::EspWifi,
    nvs::EspDefaultNvsPartition,
    eventloop::EspSystemEventLoop,
};
use esp_println::println;
use heapless::String;
use mqtt::control::ConnectReturnCode;
use mqtt::packet::{ConnackPacket, ConnectPacket, PublishPacketRef, QoSWithPacketIdentifier};
use mqtt::{Decodable, Encodable, TopicName};


static TEMP_STACK_SIZE:usize = 2000;
const WIFI_SSID:&'static str=env!("WIFI_SSID");
const WIFI_PW:&'static str=env!("WIFI_PW");
const MQTT_ID:&'static str=env!("MQTT_ID");
const MQTT_PW:&'static str=env!("MQTT_PW");
const MQTT_IP:&'static str=env!("MQTT_IP");
const MQTT_TOPIC:&'static str=env!("MQTT_TOPIC");
const MQTT_ADDR:&'static str=env!("MQTT_ADDR");

const addr_hex_code: u16 = 0xA90F;
const cmd_hex_code: u16 = 0xA90F;


fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();
    let peripherals = Peripherals::take()?;
    let ir_config = TransmitConfig::new().clock_divider(80);
    let channel = peripherals.rmt.channel0;
    let ir_pin = peripherals.pins.gpio4;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let dht_pin = PinDriver::input_output_od(peripherals.pins.gpio5.downgrade())?;
    let (tx, rx)=bounded(1);

    let mut ir_tx = TxRmtDriver::new(channel, ir_pin, &ir_config)?;
    let ticks_hz = ir_tx.counter_clock()?;
    let mut test = rmt_encoder_t::default();
    
    
    let high = Pulse::new(PinState::High, PulseTicks::new(9000)?);
    let low= Pulse::new(PinState::High, PulseTicks::new(4500)?);
    let end_high = Pulse::new(PinState::High, PulseTicks::new(560)?);
    let end_low= Pulse::new(PinState::Low, PulseTicks::new(0)?);

    let sig1 = Pulse::new(PinState::High, PulseTicks::new(560)?);
    let sig2 = Pulse::new(PinState::Low, PulseTicks::new(1690)?);
    
    let sig_1 = Pulse::new(PinState::High, PulseTicks::new(560)?);
    let sig_2 = Pulse::new(PinState::Low, PulseTicks::new(0)?);

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
        .spawn(move||dht11_thread_fuction(&mut dht11, tx));

    let ir_thread = std::thread::Builder::new()
        .stack_size(TEMP_STACK_SIZE)
        .spawn(move||loop{
            send_nec_command(&mut ir_tx, 0x6681, 0x7E81).unwrap();
            FreeRtos::delay_ms(2000);
            
        });

    FreeRtos::delay_ms(4000);
    let mut mqtt_stream = mqtt_connect(&wifi_driver)?;
    loop {
        if let Ok(data)=rx.try_recv(){
            // println!("IP info: {:?}", wifi_driver.sta_netif().get_ip_info()?);
            let message = format!("{:?}",data);
            println!("{:?}",message);
            mqtt_publish(
                &wifi_driver,
                &mut mqtt_stream,
                message.as_str()
            )?;
        }
        FreeRtos::delay_ms(1);
    }
}

fn dht11_thread_fuction(dht11: &mut Dht11<PinDriver<AnyIOPin, InputOutput>>, tx:crossbeam_channel::Sender<Vec<f32>>){
    loop{
        let mut dht11_delay = Ets;
        match dht11.perform_measurement(&mut dht11_delay) {
            Ok(measurement) =>{
                let mut sens_list = vec![];
                sens_list.push(measurement.temperature as f32 / 10.0);
                sens_list.push(measurement.humidity as f32 / 10.0);
                if let Ok(_)=tx.send(sens_list){
                    println!("{},{}",
                    measurement.temperature as f32 / 10.0,
                    measurement.humidity as f32 / 10.0);
                }
            }
                ,
            Err(e)=>
                println!("{:?}",e)
        }
        FreeRtos::delay_ms(2000);
    }
}


fn mqtt_connect(_: &EspWifi) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(MQTT_ADDR)?;
    let mut conn = ConnectPacket::new("ESP");
    conn.set_clean_session(true);
    conn.set_user_name(Some(MQTT_ID.into()));
    conn.set_password(Some(MQTT_PW.into()));
    let mut buf = Vec::new();
    conn.encode(&mut buf)?;
    stream.write_all(&buf[..])?;
    let conn_ack = ConnackPacket::decode(&mut stream)?;
    if conn_ack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {
        println!("MQTT failed to receive the connection accepted ack");
    }
    println!("MQTT connected");

    Ok(stream)
}


fn mqtt_publish(
    _: &EspWifi,
    stream: &mut TcpStream,
    message: &str,
) -> anyhow::Result<()> {
    let topic = unsafe { TopicName::new_unchecked(MQTT_TOPIC.to_string()) };
    let bytes = message.as_bytes();
    let publish_packet = PublishPacketRef::new(&topic, QoSWithPacketIdentifier::Level0, bytes);
    let mut buf = Vec::new();
    publish_packet.encode(&mut buf)?;
    stream.write_all(&buf[..])?;
    Ok(())
}


fn send_nec_command(
    ir_tx: &mut TxRmtDriver,
    address: u16,
    command: u16,
) -> anyhow::Result<()> {
    // NEC 프로토콜의 타이밍 정의
    let start_high = Pulse::new(PinState::High, PulseTicks::new(9000)?);
    let start_low = Pulse::new(PinState::Low, PulseTicks::new(4500)?);
    let bit_high = Pulse::new(PinState::High, PulseTicks::new(560)?);
    let bit_low_0 = Pulse::new(PinState::Low, PulseTicks::new(560)?);
    let bit_low_1 = Pulse::new(PinState::Low, PulseTicks::new(1690)?);
    let end_high = Pulse::new(PinState::High, PulseTicks::new(560)?);

    // NEC 프로토콜 신호 설정 (32비트: Address + Command)
    let mut signal = FixedLengthSignal::<67>::new();
    
    // 시작 신호
    signal.set(0, &(start_high, start_low))?;
    
    // 16비트 Address 신호 전송
    for i in (0..16).rev() {
        let bit = (address >> i) & 0x1;
        if bit == 1 {
            signal.set(i as usize + 1, &(bit_high, bit_low_1))?;
        } else {
            signal.set(i as usize + 1, &(bit_high, bit_low_0))?;
        }
    }

    // 16비트 Command 신호 전송
    for i in (0..16).rev() {
        let bit = (command >> i) & 0x1;
        if bit == 1 {
            signal.set(i as usize + 17, &(bit_high, bit_low_1))?;
        } else {
            signal.set(i as usize + 17, &(bit_high, bit_low_0))?;
        }
    }
    
    // 끝 신호
    signal.set(66, &(end_high, Pulse::default()))?;
    
    // 신호 전송
    ir_tx.start_blocking(&signal)?;
    
    Ok(())
}

// fn send_nec_command(
//     ir_tx: &mut TxRmtDriver,
//     address: u16,
//     command: u16,
// ) -> anyhow::Result<()> {
//     // NEC 프로토콜의 타이밍 정의
//     let start_high = Pulse::new(PinState::High, PulseTicks::new(9000)?);
//     let start_low = Pulse::new(PinState::Low, PulseTicks::new(4500)?);
//     let bit_high = Pulse::new(PinState::High, PulseTicks::new(560)?);
//     let bit_low_0 = Pulse::new(PinState::Low, PulseTicks::new(560)?);
//     let bit_low_1 = Pulse::new(PinState::Low, PulseTicks::new(1690)?);
//     let end_high = Pulse::new(PinState::High, PulseTicks::new(560)?);

//     // VariableLengthSignal 사용으로 유연하게 신호 설정
//     let mut signal = VariableLengthSignal::new();
    
//     // 시작 신호 추가
//     signal.push(&[start_high, start_low])?;
    
//     // Address 비트 전송
//     for i in (0..16).rev() {
//         let bit = (address >> i) & 0x1;
//         if bit == 1 {
//             signal.push(&[bit_high, bit_low_1])?;
//         } else {
//             signal.push(&[bit_high, bit_low_0])?;
//         }
//     }

//     // Command 비트 전송
//     for i in (0..16).rev() {
//         let bit = (command >> i) & 0x1;
//         if bit == 1 {
//             signal.push(&[bit_high, bit_low_1])?;
//         } else {
//             signal.push(&[bit_high, bit_low_0])?;
//         }
//     }
    
//     // 끝 신호 추가
//     signal.push(&[end_high])?;
    
//     // 신호 전송
//     ir_tx.start_blocking(&signal)?;
    
//     Ok(())
// }