# Inky-Frame

Drivers and Utilities for Pinorami InkyFrame Devices

Rust library with helpers and API functions to design programs for InkyFrame
devices or take advantage of any of the supplied embedded device utilities.

## InkyFrame Devices

- [Inky Frame 4](https://shop.pimoroni.com/products/inky-frame-4?variant=40443825094739)
- [Inky Frame 5.7](https://shop.pimoroni.com/products/inky-frame-5-7?variant=40048398958675)

The Inky Frame 7.3 version uses different methods to update the display and set
color information and is not directly supported. _(I am open to pull requests to_
_add support :3)_

_While the 5.7 model is supported, I don't have one on-hand to test. Specifically,_
_the Display buffer size; it might need to be adjusted._

### Supported Devices

Support for devices matches the underlying [RPSP support library](https://github.com/secfurry/rpsp?tab=readme-ov-file#supported-devices).
View the [RPSP README](https://github.com/secfurry/rpsp/blob/main/README.md) for more information.

## Exposed Libraries

While this library is written for InkyFrame devices, the helper utilities contained
in this library can be used with any embedded device.

- UC8159 eInk SPI Driver with 7-Color Dithering
- SD Card SPI Driver
- FAT Filesystem Driver (with long filename support!)
- TGA Image Parser
- PCF85063A RTC I2C Driver

## Note

You'll need to make sure you have the `flip-link` linker installed before compiling.
To do this, use the command `cargo install flip-link` to install it.

### Additional Cargo Configuration

For best results, create a `.cargo/config.toml` file in your project root directory
and specify somthing like this:

```toml
[target.'cfg(all(target_arch = "arm", target_os = "none"))']
rustflags  = [
    "-C", "linker=flip-link",
    "-C", "link-arg=--nmagic",
    "-C", "link-arg=-Tlink.x",
    "-Z", "trap-unreachable=no",
    "-C", "no-vectorize-loops",
]

[build]
target    = "thumbv6m-none-eabi"
```

Requires the `nightly` version of the compiler to use `"-Z", "trap-unreachable=no",`
and can be removed, but will increase binary size slightly.

Extra bonus points if you add:

```toml
runner    = "probe-rs run --chip RP2040"
```

Under the `rustflags` option. This allows you to flash the firmware on the device
directly from `cargo run`. (Pico debug probe probe and `probe-rs` required. Can be
installed using `cargo install probe-rs-tools`. Pico probe can be made from another
Pico! [see here](https://mcuoneclipse.com/2022/09/17/picoprobe-using-the-raspberry-pi-pico-as-debug-probe/)).

Lastly, these are the recommended profile settings for best program results. These
go inside the `Cargo.toml` file in the project root directory.

```toml
[profile.dev]
debug            = 2
strip            = false
opt-level        = 3
incremental      = false
codegen-units    = 1
overflow-checks  = true
debug-assertions = true

[profile.release]
lto              = "fat"
panic            = "abort"
debug            = false
strip            = true
opt-level        = 3
incremental      = false
codegen-units    = 1
overflow-checks  = false
debug-assertions = false
```

## Usage

To use this library, just import `inky_frame::InkyBoard` and call `InkyBoard::get()`.
On the first call, the device and it's clocks will be initialized and setup fully.

The configuration is automatic and uses the ROSC as the system clock, disables
the XOSC and PLLs and allows for DORMANT sleep, for maximum power savings.

_This is similar to the RPSP `Board::get()` setup and init behavior. The `InkyBoard`_
_struct actually has `Deref` for the `Board` struct and can be used as a drop-in replacement._

To supply main, you must setup the `main` function with the `#[rpsp::entry]` macro,
which will setup the locks and properly redirect execution to the selected function.

Basic programs should look something like this:

```rust
#![no_std]
#![no_main]

#[rpsp::entry]
fn main() -> ! {
    // do stuff here
}
```

If you're not using something like `defmt`, _(which is not included by default)_
you'll need a `panic_handler`. The example below is a pretty basic one that just
loops the CPU:

```rust
#[panic_handler]
fn panic(_p: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
```

For the below examples, the `panic_handler` is omitted, so if you want to use
these, you'll need to add it in order for it to compile.

## Inky Examples

### InkyBoard

The `InkyBoard` struct exposes many helper functions to use the default peripherals
on the board.

```rust
let i = InkyBoard::get();

i.sync_rtc_to_pcf(); // Same as the Python function "pico_rtc_to_pcf"
i.sync_pcf_to_rtc(); // Same as the Python function "pcf_to_pico_rtc"

// Same as the Python function "sleep_for"
// THIS IS IN SECONDS!! Instead of minutes
// Requires unsafe since it will poweroff the Pico
unsafe { i.deep_sleep(60) };

// Same as the Python function "turn_off"
// Requires unsafe since it will poweroff the Pico
unsafe { i.power_off() };

let w = i.wake_reason(); // Receives the reason the Pico was woken.

w.wake_from_rtc(); // Same as the Python function "woken_by_rtc"
w.wake_from_ext(); // Same as the Python function "woken_by_ext_trigger"
w.wake_from_button(); // Same as the Python function "woken_by_button"

let pcf = i.pcf(); // Pointer to the PCF85063A RTC

pcf.set_byte(1).unwrap(); // Exposes the free 1-byte register of the PCF
let _ = pcf.get_byte().unwrap();

// Pointer to the I2C bus
// This is the bus the PCF is running on.
let i2c_bus = i.i2c_bus();

// Pointer to the SPI bus
// This is the bus the SDCard and Inky are running on.
// The bus is not initialized until this is first called.
let spi_bus = i.spi_bus();

// Create the SDCard and wrap it with the FAT filesystem driver
// Will initialize the SPI bus.
// This is not owned by the "InkyBoard" struct, so multiple calls to this will
// attempt to recreate it.
let sd = i.sd_card();
```

### Buttons

```rust
use inky_frame::{InkyBoard, buttons};

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();
    let b = p.buttons(); // Pointer to Buttons, owned by 'p'.
    // Button data must be manually refreshed by calling "b.read()".
    // On the first read (done automatically), the pressed buttons will be
    // the same as the Wake Reason.

    // Is button 'A' pressed?
    if b.button_a() {
        // Do stuff..
    }
    // Is button 'B' pressed?
    if b.button_b() {
        // Do stuff..
    }
    // Is button 'C' pressed?
    if b.button_c() {
        // Do stuff..
    }
    // Is button 'E' pressed?
    if b.button_d() {
        // Do stuff..
    }
    // Is button 'E' pressed?
    if b.button_e() {
        // Do stuff..
    }

    // Returns an enum of what was pressed. This also covers external triggers
    // like the RTC and External Trigger.
    let button = b.pressed();

    // Another way to read the button ShiftRegister
    // Returns true if a Button or the External Trigger was the cause.
    let b = b.read_pressed();

    // Access a reference to the ShiftRegister
    // The ShiftRegister can be cloned if ownership is needed.
    let sr = b.shift_register();

    // If you need ownership of the Buttons struct, you can call
    let ob = buttons();
    // Has the same functions as the Buttons reference.

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### Leds

```rust
use inky_frame::{InkyBoard, leds};

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();
    let l = p.leds(); // Pointer to Leds, owned by 'p'.

    l.all_on(); // Turn all LEDs on.
    l.all_off(); // Turn all LEDs off.

    l.a.on(); // Set Button 'A' LED on.
    l.b.on(); // Set Button 'B' LED on.
    l.c.on(); // Set Button 'C' LED on.
    l.d.on(); // Set Button 'D' LED on.
    l.e.on(); // Set Button 'E' LED on.

    l.network.on(); // Set the Network (Top Right) LED on.
    l.activity.on(); // Set the Activity (Top Left) LED on.

    // The LEDs are PWM LEDs, which means their brightness can be adjusted also.
    l.all_brightness(50); // Set brightness of all LEDs to 50%.

    l.a.brightness(25); // Set Button 'A' LED brightness to 25%.

    // Similar to the Button struct, if you need ownership of the LEDs struct, you
    // can call
    let ol = leds();
    // Has the same functions as the LEDs reference.

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### PCF RTC

```rust
use inky_frame::InkyBoard;

use rpsp::clock::AlarmConfig;
use rpsp::time::{Month, Time, Weekday};

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();
    let r = p.pcf(); // Pointer to the PCF RTC, owned by 'p'.

    let now = r.now().unwrap(); // Read the PCF's current time.

    // Create new Time struct.
    let dt = Time::new(2025, Month::March, 1, 15, 30, 0, Weekday::Tuesday);

    // Set the PCF's current time
    r.set_time(dt).unwrap();

    // Set the PCF RTC Alarm
    r.set_alarm(AlarmConfig::new().hour(16).mins(0).secs(0)).unwrap();
    r.set_alarm_interrupt(true).unwrap();

    // Use the free PCF 1-byte register
    // Set it
    r.set_byte(190).unwrap();
    // Retrive it
    let _ = r.get_byte();

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### SD Card and FAT File System

```rust
use inky_frame::fs::Storage;
use inky_frame::sd::Card;
use inky_frame::InkyBoard;

use rpsp::pin::PinID;

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();

    // There's two ways to make the SD Card. We'll use the more complex way.
    let mut sd = Storage::new(Card::new(&p, PinID::Pin22, p.spi_bus()))
    // let mut sd = p.sd_card(); // Easy way.

    let mut r = sd.root().unwrap(); // Get the first Volume on the SD Card.

    // Open the "root" directory (/)
    let root = r.dir_root();

    // List the entries in the root directory.
    root.list().unwrap().iter(|x| {
        // Called on each entry..
        if x.is_file() {
            debug!("Found file {}!", x.name());
            debug!("Size is {}b", x.size());
        } else {
            debug!("Found directory {}!", x.name());
        }
    });

    // Open a file under the "root" directory
    let mut my_file_1 = root.file("test_file1.txt", false).unwrap();
    my_file_1.close();


    // Open a file recursively (like an OS)
    let mut f = r.open("/my_dir1/my_dir2/my_filename.txt").unwrap();

    // Read some data from it
    let mut buf = [0u8; 32];
    let n = f.read(&mut buf).unwrap();
    debug!("Read {n} bytes: [:?]", &buf[0..n]);

    // Create a new file
    let mut my_file_2 = r.file_create("/my_dir3/testing_dir2/file.txt").unwrap();
    // This will create the directory structure if it does not exist.
    // Write to the file
    let _ = my_file_2.write("Hello There!\n".as_bytes()).unwrap();
    // Flush out the data to the card.
    my_file_2.close();

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### Inky and TGA Image Parsing

```rust
use inky_frame::frame::tga::TgaParser;
use inky_frame::frame::{Color, Inky, InkyPins, InkyRotation, RGB};
use inky_frame::fs::Storage;
use inky_frame::sd::Card;
use inky_frame::InkyBoard;

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();

    // We're gonna use the InkyFrame4 for this example.
    // The signature of the "Inky" struct is
    //   pub struct Inky<'a, const B: usize, const W: u16, const H: u16> {}
    // Where:
    // - B is the buffer size (InkyFrame4 is 128_000)
    // - W is the screen width (InkyFrame4 is 640)
    // - H is the screen height (InkyFrame4 is 400)
    //
    // The alias "Inky4" can be used for convienence. It's singature is:
    //   pub type Inky4<'a> = Inky<'a, 128_000, 640u16, 400u16>
    //
    // 'InkyPins::inky_frame4()' describes the default SPI and Busy pin
    // configuration. When using "new" the SPI details are ignored as we're
    // not creating the SPI bus.
    let mut dis = Inky4::new(&p, p.spi_bus(), InkyPins::inky_frame4()).unwrap();

    // Get the SD Card
    let mut sd = p.sd_card();
    let mut r = sd.root().unwrap(); // Get the first Volume on the SD Card.

    dis.set_fill(Color::Green); // Fill the entire screen with Green.

    // Fill a rectangle from 200,50 to 300,100 with Blue.
    for x in 200..300 {
        for y in 50..100 {
            dis.set_pixel(x, y, Color::Blue);
        }
    }

    // Fill a rectangle from 400,100 to 500,200 with "#C783E8".
    // Dithering will be applied to the color to make it appear as close as
    // possible to the real color.
    for x in 400..500 {
        for y in 100..200 {
            dis.set_pixel_color(x, y, RGB::rgb(199, 131, 232));
        }
    }

    // Update the display to show the changes.
    // This function blocks until the refresh is done.
    dis.update();

    // The 'set_with' command will run function with itself as the function
    // argument to prevent any mutable contention issues. This function can
    // pass back any errors with "?".
    //
    // Also is good for limiting resource usage inside the closure.
    dis.set_with(|x| {
        let mut f = r.open("/my_image1.tga").unwrap();
        // Load the image into the TGA parser.
        let img = TgaParser::new(&mut f)?;
        // Set the image at 50,50
        x.set_image(50, 50, img)
    }).unwrap();

    // TGA supports transparency and any transparent pixels will be skipped to
    // allow for transparent overlapping images to be loaded.
    dis.set_with(|x| {
        let mut f = r.open("/my_image2.tga").unwrap();
        // Load the image into the TGA parser and set the image at 0,0
        x.set_image(0, 0, TgaParser::new(&mut f)?)
    }).unwrap();

    // Update the display
    dis.update();

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### Low Power Shutoff on Battery

If the JST connector is used, the Frame can power off the Pico and wake it
up when a button is pressed or by the PCF RTC alarm, allowing for super low
power usage.

```rust
use inky_frame::InkyBoard;

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();

    unsafe { p.deep_sleep(30) };
    // This will set the PCF RTC alarm for 30 seconds from now and power off
    // the Pico if the JST Battery connector is used. After 30 seconds, the
    // Pico will boot back up (like a reboot).
    //
    // If not using the JST Battery connector, this will just wait 30 seconds
    // instead, then return.

    unsafe { p.power_off() };
    // Directly power off the Pico.
    //
    // Code after this will not run unless the JST Battery connector is not being
    // used.

    debug!("Not on battery!");

    // Need this at the end since it's a '!' function.
    loop {}
}
```

## Standard Examples

These are taken from [here](https://github.com/secfurry/rpsp/blob/main/README.md),
but are modified to show the drop-in replacement of the `InkyBoard` struct.

### GPIO

Control Pin output:

```rust
use inky_frame::InkyBoard;

use rpsp::pin::PinID;

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();
    let my_pin = p.pin(PinID::Pin5);
    // You could also do..
    // let my_pin = Pin::get(&p, PinID::Pin5);

    // Set High
    my_pin.high();

    // Set Low
    my_pin.low();

    // Need this at the end since it's a '!' function.
    loop {}
}
```

Read Pin output:

```rust
use inky_frame::InkyBoard;

use rpsp::pin::PinID;

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();
    let my_pin = p.pin(PinID::Pin6).into_input();

    // Set High
    if my_pin.is_high() {
        // Do stuff..
    }

    if my_pin.is_low() {
        // Do other stuff..
    }

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### UART

```rust
use inky_frame::InkyBoard;

use rpsp::pin::PinID;
use rpsp::uart::{Uart, UartConfig, UartDev};

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();

    // DEFAULT_BAUDRATE is 115,200
    let mut u = Uart::new(
        &p,
        UartConfig::DEFAULT_BAUDRATE,
        UartConfig::new(), // Default is NoParity, 8 Bits, 1 Stop Bit.
        UartDev::new(PinID::Pin0, PinID::Pin1).unwrap(),
        // ^ This can error since not all Pinouts are a valid UART set.
        // You can also use..
        // (PinID::Pin0, PinID::Pin1).into()
    ).unwrap();

    let _ = u.write("HEY THERE\n".as_bytes()).unwrap();
    // Returns the amount of bytes written.

    let mut buf = [0u8; 32];
    let n = u.read(&mut buf).unwrap();
    // Read up to 32 bytes.

    // Echo it back.
    let _ = u.write(&buf[0:n]).unwrap();

    // Cleanup
    u.close();

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### Time  and Sleep

```rust
use inky_frame::InkyBoard;

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();

    for i in 0..25 {
        p.sleep(5_000); // Wait 5 seconds.

        // Get current RTC time.
        let now = p.rtc().now().unwrap();

        debug!("the time is now {now:?}");
    }

    // Need this at the end since it's a '!' function.
    loop {}
}
```

### Watchdog

```rust
use inky_frame::InkyBoard;

#[rpsp::entry]
fn main() -> ! {
    let p = InkyBoard::get();
    let dog = p.watchdog();

    dog.start(5_000); // Die if we don't feed the dog every 5 seconds.

    for _ in 0..10 {
        p.sleep(2_500); // ait 2.5 seconds.

        dog.feed(); // Feed da dog.
    }

    p.sleep(10_000); // Device will restart during here.

    // Need this at the end since it's a '!' function.
    loop {}
}
```

## Bugs

Some SDCards don't support SPI mode or don't initialize properly. I'm not
100% sure if it's a protocol issue or something else. These cards return
`READY (0)` when asked to go into `IDLE (1)` mode. They'll work fine on PCs.

These SDCards work fine on some of my Ender 3 3D Printers, _which use Arduino's_
_SDCard library_ and have the same initializing sequence. But other devices, like
the Flipper Zero, don't work with them either.

You'll know if it fails as it won't get past the initialization phase and basically
"freezes" and does not respond with the "D" and "E" LEDs on. __This error type__
__does not use the Activity or Network LEDs.__

If you have a SDCard that has issues also, please leave an [Issue](https://github.com/secfurry/inky-frame/issues/new?title=SDCard+Init+Critical)
with information on the SDCard and it's manufactor, size and class markings
(eg: C with the number, U with the number, etc.) so I can test further.

SDCards verified to __not__ work:

- [These SDHC Class 10/U1 Cards](https://www.amazon.com/dp/B07XJVFVSJ)
