#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

// 1. Define the physical inputs from the "field" (Sensors & Switches)
typedef struct {
    uint16_t temperature_celsius;
    uint16_t pressure_psi;
    uint8_t  flow_rate;
    uint8_t  command_flags; // Bit 0: Start, Bit 1: Stop, Bit 2: Maintenance Override
} SensorInputs;

// 2. Define the PLC's internal memory/state
typedef struct {
    uint8_t  system_running;
    uint8_t  error_state;
    uint16_t heat_accumulator; // Tracks how long the system has been running hot
} PlcState;

// 3. The actual PLC Control Logic (What you are trying to break)
void execute_plc_scan_cycle(const SensorInputs* inputs, PlcState* state) {
    // Parse the HMI/Hardware commands
    uint8_t cmd_start    = (inputs->command_flags & 0x01) != 0;
    uint8_t cmd_stop     = (inputs->command_flags & 0x02) != 0;
    uint8_t cmd_override = (inputs->command_flags & 0x04) != 0;

    // Basic Start/Stop Latching Logic
    if (cmd_start && !cmd_stop) {
        state->system_running = 1;
    } else if (cmd_stop) {
        state->system_running = 0;
    }

    // If the system is off, cool down and exit the scan early
    if (!state->system_running) {
        state->heat_accumulator = 0;
        return;
    }

    // Accumulate heat if running hot
    if (inputs->temperature_celsius > 150) {
        // Prevent simple overflow via standard logic
        if (state->heat_accumulator < 60000) { 
            state->heat_accumulator += (inputs->temperature_celsius - 150);
        }
    } else {
        state->heat_accumulator = 0;
    }

    // --- VULNERABILITIES FOR THE FUZZER TO FIND ---

    // VULNERABILITY 1: Logic Flaw (Sensor Failure + Override)
    // If the temperature is critically high, but the pressure sensor breaks (reads 0),
    // and an operator holds the maintenance override button, it causes a catastrophic failure.
    if (inputs->temperature_celsius > 800 && inputs->pressure_psi == 0 && cmd_override) {
        // Fuzzer goal: Find this specific edge case
        abort(); 
    }

    // VULNERABILITY 2: Deep State Bug
    // The fuzzer must sequence multiple scan cycles to build up the heat_accumulator.
    // If it hits exactly 50,000 while the flow rate is mysteriously locked at a magic number (0x42), it crashes.
    if (state->heat_accumulator >= 50000 && inputs->flow_rate == 0x42) {
        // Fuzzer goal: Figure out how to hold the system in a hot state without 
        // triggering a shutdown, then inject a specific flow rate.
        abort();
    }
}

// 4. The Fuzzer Harness (The Entry Point)
// Instead of treating the fuzzer input as one big file, we slice it into chunks.
// Each chunk represents the sensor readings for a single PLC scan cycle.
int LLVMFuzzerTestOneInput(const uint8_t *Data, size_t Size) {
    // We need at least enough bytes for one scan cycle
    if (Size < sizeof(SensorInputs)) {
        return 0; 
    }

    // Boot up the PLC (Initialize state to zero)
    PlcState current_state;
    memset(&current_state, 0, sizeof(PlcState));

    // Time-Series Simulation: 
    // Loop through the fuzzer's byte array, feeding it to the PLC one scan cycle at a time.
    size_t offset = 0;
    while (offset + sizeof(SensorInputs) <= Size) {
        SensorInputs current_inputs;
        
        // Safely map the raw fuzzer bytes into our structured PLC inputs
        memcpy(&current_inputs, Data + offset, sizeof(SensorInputs));
        
        // Execute one tick of the PLC loop
        execute_plc_scan_cycle(&current_inputs, &current_state);
        
        // Move to the next "tick" in the fuzzer's data stream
        offset += sizeof(SensorInputs);
    }

    return 0; // Execution successful, keep fuzzing
}