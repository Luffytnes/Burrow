// burrow-smc — SMC fan controller for Burrow (Apple Silicon + Intel)
// Build: swiftc burrow-smc.swift -o burrow-smc
// Usage (requires sudo): burrow-smc apply <0|1> <0-100>
//   apply 0 0   → auto (release SMC override)
//   apply 1 60  → manual at 60% of max fan speed

import Foundation
import IOKit

// MARK: - SMC constants

private let kSMCKeyNotFound: UInt8 = 0x84
private let kSMCSuccess:     UInt8 = 0x00

private let KERNEL_INDEX_SMC: UInt32 = 2
private let SMC_CMD_READ_KEYINFO: UInt8 = 9
private let SMC_CMD_READ_BYTES:   UInt8 = 5
private let SMC_CMD_WRITE_BYTES:  UInt8 = 6

// MARK: - SMC data structures (must match kernel layout exactly)

private struct SMCVersion {
    var major:    UInt8  = 0
    var minor:    UInt8  = 0
    var build:    UInt8  = 0
    var reserved: UInt8  = 0
    var release:  UInt16 = 0
}

private struct SMCPLimitData {
    var version:   UInt16 = 0
    var length:    UInt16 = 0
    var cpuPLimit: UInt32 = 0
    var gpuPLimit: UInt32 = 0
    var memPLimit: UInt32 = 0
}

private struct SMCKeyInfoData {
    var dataSize:       UInt32                = 0
    var dataType:       UInt32                = 0
    var dataAttributes: UInt8                 = 0
}

private struct SMCKeyData {
    var key:        UInt32        = 0
    var vers:       SMCVersion    = SMCVersion()
    var pLimitData: SMCPLimitData = SMCPLimitData()
    var keyInfo:    SMCKeyInfoData = SMCKeyInfoData()
    var result:     UInt8         = 0
    var status:     UInt8         = 0
    var data8:      UInt8         = 0
    var data32:     UInt32        = 0
    var bytes: (UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,
                UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,
                UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,
                UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8,UInt8) =
        (0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0)
}

// MARK: - SMC helpers

private func fourCC(_ s: String) -> UInt32 {
    var result: UInt32 = 0
    for scalar in s.unicodeScalars.prefix(4) {
        result = (result << 8) | scalar.value
    }
    return result
}

private func toFloat(_ bytes: (UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8,
                               UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8,
                               UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8,
                               UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8, UInt8),
                    type: String) -> Float {
    // fpe2: 14.2 fixed point (fan speed in rpm × 4)
    if type == "fpe2" || type == "{fpe" {
        let raw = UInt16(bytes.0) << 8 | UInt16(bytes.1)
        return Float(raw) / 4.0
    }
    // sp78: 8.8 signed fixed point
    if type == "sp78" {
        let raw = Int16(bitPattern: UInt16(bytes.0) << 8 | UInt16(bytes.1))
        return Float(raw) / 256.0
    }
    // ui8 / ui16 / ui32
    if type.hasPrefix("ui") {
        return Float(bytes.0)
    }
    return 0
}

// MARK: - SMC connection

private class SMC {
    private var connection: io_connect_t = 0

    init?() {
        var service: io_service_t = IOServiceGetMatchingService(
            kIOMainPortDefault,
            IOServiceMatching("AppleSMC")
        )
        guard service != 0 else { return nil }
        defer { IOObjectRelease(service) }
        guard IOServiceOpen(service, mach_task_self_, 0, &connection) == kIOReturnSuccess else { return nil }
    }

    deinit { IOServiceClose(connection) }

    private func call(_ inputStruct: inout SMCKeyData, _ outputStruct: inout SMCKeyData) -> Bool {
        var inputSize  = MemoryLayout<SMCKeyData>.size
        var outputSize = MemoryLayout<SMCKeyData>.size
        let result = IOConnectCallStructMethod(
            connection,
            UInt32(KERNEL_INDEX_SMC),
            &inputStruct, inputSize,
            &outputStruct, &outputSize
        )
        return result == kIOReturnSuccess
    }

    func readKeyInfo(_ key: UInt32) -> SMCKeyInfoData? {
        var input  = SMCKeyData()
        var output = SMCKeyData()
        input.key    = key
        input.data8  = SMC_CMD_READ_KEYINFO
        guard call(&input, &output), output.result == kSMCSuccess else { return nil }
        return output.keyInfo
    }

    func readKey(_ key: String) -> (bytes: SMCKeyData, type: String)? {
        let k = fourCC(key)
        guard let info = readKeyInfo(k) else { return nil }
        var input  = SMCKeyData()
        var output = SMCKeyData()
        input.key            = k
        input.keyInfo        = info
        input.data8          = SMC_CMD_READ_BYTES
        guard call(&input, &output), output.result == kSMCSuccess else { return nil }
        let t = withUnsafeBytes(of: info.dataType.bigEndian) { buf in
            String(bytes: buf.prefix(4), encoding: .ascii) ?? ""
        }
        return (output, t)
    }

    func readFloat(_ key: String) -> Float? {
        guard let (data, type) = readKey(key) else { return nil }
        return toFloat(data.bytes, type: type.trimmingCharacters(in: .controlCharacters))
    }

    func writeUI8(_ key: String, value: UInt8) -> Bool {
        let k = fourCC(key)
        guard let info = readKeyInfo(k) else { return false }
        var input  = SMCKeyData()
        var output = SMCKeyData()
        input.key      = k
        input.keyInfo  = info
        input.data8    = SMC_CMD_WRITE_BYTES
        input.bytes.0  = value
        return call(&input, &output) && output.result == kSMCSuccess
    }

    func writeFpe2(_ key: String, rpmValue: Float) -> Bool {
        let k = fourCC(key)
        guard let info = readKeyInfo(k) else { return false }
        var input  = SMCKeyData()
        var output = SMCKeyData()
        input.key     = k
        input.keyInfo = info
        input.data8   = SMC_CMD_WRITE_BYTES
        // fpe2 = value × 4, big-endian 16-bit
        let raw = UInt16(rpmValue * 4.0)
        input.bytes.0 = UInt8(raw >> 8)
        input.bytes.1 = UInt8(raw & 0xFF)
        return call(&input, &output) && output.result == kSMCSuccess
    }
}

// MARK: - Fan control

func applyFanMode(mode: Int, percent: Int) -> Bool {
    guard let smc = SMC() else {
        fputs("error: cannot open AppleSMC\n", stderr); return false
    }

    let numFans = Int(smc.readFloat("FNum") ?? 1.0)

    if mode == 0 {
        // Auto: release manual control
        for i in 0..<numFans {
            let fsKey = "FS\(i)!"
            _ = smc.writeUI8(fsKey, value: 0)
        }
        print("auto: released fan override")
        return true
    }

    // Manual: set minimum speed to percent% of max
    for i in 0..<numFans {
        let maxKey  = "F\(i)Mx"
        let mnKey   = "F\(i)Mn"
        let fsKey   = "FS\(i)!"

        let maxRPM  = smc.readFloat(maxKey) ?? 6000.0
        let target  = maxRPM * Float(percent) / 100.0

        // Enable manual control
        _ = smc.writeUI8(fsKey, value: 1)
        // Set minimum speed (forces fan to spin at least this fast)
        _ = smc.writeFpe2(mnKey, rpmValue: target)

        print("fan \(i): set to \(Int(target)) RPM (\(percent)% of \(Int(maxRPM)))")
    }
    return true
}

// MARK: - Main

func printUsage() {
    print("Usage: burrow-smc apply <0|1> <0-100>")
    print("  apply 0 0   → auto mode")
    print("  apply 1 60  → manual at 60%")
}

let args = CommandLine.arguments
if args.count == 4 && args[1] == "apply",
   let mode    = Int(args[2]),
   let percent = Int(args[3]),
   (mode == 0 || mode == 1),
   (0...100).contains(percent)
{
    exit(applyFanMode(mode: mode, percent: percent) ? 0 : 1)
} else {
    printUsage()
    exit(1)
}
