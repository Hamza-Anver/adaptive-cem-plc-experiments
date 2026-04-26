#!/usr/bin/env python3
"""
Test script to verify Python bindings can access C program variables via Rust lib
"""

import sys
from libafl_sandbox import TargetSession

def main():
    print("=" * 60)
    print("LibAFL Sandbox - Python Binding Test")
    print("=" * 60)
    
    try:
        # Create and boot target session
        print("\n[1] Creating TargetSession...")
        session = TargetSession()
        print(f"    Session created: {session}")
        
        print("\n[2] Booting target...")
        session.boot()
        print("    Target booted successfully")
        
        # Test input size
        print("\n[3] Reading input size...")
        input_sz = session.input_size()
        print(f"    Input size: {input_sz} bytes")
        
        # Test full state size
        print("\n[4] Reading full state size...")
        state_sz = session.state_size()
        print(f"    State size: {state_sz} bytes")
        
        # Test reading variable metadata (this is the main test)
        print("\n[5] Reading C program variable metadata...")
        var_metadata = session.var_metadata()
        print(f"    Found {len(var_metadata)} variables:")
        
        if var_metadata:
            print("\n    Variables:")
            for var in var_metadata:
                print(f"      - {var}")
        else:
            print("      (No variables returned)")

        print("\n[6] Reading input hints...")
        input_hints = session.input_hints()
        print(f"    Found {len(input_hints)} input hints")
        if len(input_hints) != input_sz:
            raise RuntimeError("input_hints count should match input_size for byte-oriented targets")
        if input_hints:
            for hint in input_hints:
                print(f"      - {hint}")

        print("\n[7] Reading variables as a dict...")
        vars_before = session.read_vars()
        print(f"    Read {len(vars_before)} variables via dict API")

        print("\n[8] Setting one variable from dict...")
        phase_before = int(vars_before["phase"])
        phase_after = phase_before + 1
        session.write_vars({"phase": phase_after})
        vars_after = session.read_vars(["phase"])
        if int(vars_after["phase"]) != phase_after:
            raise RuntimeError("phase was not updated correctly")
        print(f"    phase changed from {phase_before} to {phase_after}")
        
        print("\n" + "=" * 60)
        print("All tests passed! Python bindings working correctly.")
        print("=" * 60)
        return 0
        
    except Exception as e:
        print(f"\n Error: {e}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        return 1

if __name__ == "__main__":
    sys.exit(main())
