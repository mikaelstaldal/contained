docker run \
  --mount type=bind,source=/lib64,target=/lib64,readonly \ 
  --mount type=bind,source=/usr,target=/usr,readonly \ 
  --mount type=bind,source=/lib,target=/lib,readonly \
  --mount type=bind,source=/home/mikes/src/rust/hello_world,target=/home/mikes/src/rust/hello_world,readonly \
  --entrypoint /home/mikes/src/rust/hello_world/main \
  empty
