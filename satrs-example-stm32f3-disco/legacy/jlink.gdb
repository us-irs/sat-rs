target extended-remote localhost:2331

monitor reset

# *try* to stop at the user entry point (it might be gone due to inlining)
break main

load

continue
