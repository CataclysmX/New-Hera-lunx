// Ghidra headless post-script: decompile every non-thunk function to one C file.
// Usage: analyzeHeadless <projDir> <proj> -import <bin> -scriptPath tools/re
//        -postScript DecompileAll.java <out.c> -deleteProject
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;

import java.io.FileWriter;
import java.io.PrintWriter;

public class DecompileAll extends GhidraScript {
    @Override
    public void run() throws Exception {
        String out = getScriptArgs()[0];
        DecompInterface d = new DecompInterface();
        d.openProgram(currentProgram);
        try (PrintWriter w = new PrintWriter(new FileWriter(out))) {
            for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
                if (f.isExternal() || f.isThunk()) {
                    continue;
                }
                DecompileResults r = d.decompileFunction(f, 120, monitor);
                w.println("// ==== " + f.getName() + " @ " + f.getEntryPoint());
                if (r.decompileCompleted()) {
                    w.println(r.getDecompiledFunction().getC());
                } else {
                    w.println("// decompile failed: " + r.getErrorMessage());
                }
            }
        }
        d.dispose();
    }
}
