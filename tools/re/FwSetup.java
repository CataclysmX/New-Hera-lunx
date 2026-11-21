// Ghidra headless pre-script for the raw STM32 firmware image (base 0x08000000, Thumb-only Cortex).
// Creates functions at every vector-table handler and enables the aggressive instruction finder.
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;

public class FwSetup extends GhidraScript {
    @Override
    public void run() throws Exception {
        setAnalysisOption(currentProgram, "ARM Aggressive Instruction Finder", "true");
        Address base = toAddr(0x08000000L);
        for (int i = 1; i < 0x80; i++) {
            long v = getInt(base.add(i * 4L)) & 0xffffffffL;
            if ((v & 1) == 1 && v >= 0x08000000L && v < 0x08200000L) {
                Address a = toAddr(v & ~1L);
                disassemble(a);
                if (getFunctionAt(a) == null) {
                    createFunction(a, "vec_" + i);
                }
            }
        }
    }
}
